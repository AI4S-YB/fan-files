use crate::llm::LlmClient;
use crate::project::ProjectStore;
use std::collections::HashMap;
use tracing::{info, warn};

/// Run the full LLM inference pipeline
pub fn run_inference(
    sqlite: &crate::index::sqlite::SqliteStore,
    project_store: &ProjectStore,
    llm_client: &LlmClient,
    scan_root: &str,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        info!("LLM not configured, skipping inference");
        return Ok((0, 0));
    }

    // 1. Collect directory tree from indexed files
    let dirs = collect_directory_summary(sqlite);
    if dirs.is_empty() {
        info!("No directories found for inference");
        return Ok((0, 0));
    }

    // 2. Build prompt and call LLM
    let summary = crate::llm::prompt::build_directory_summary(scan_root, &dirs);
    info!(
        "Sending {} directories to LLM for inference...",
        dirs.len()
    );

    let output = match llm_client.infer_projects(&summary) {
        Ok(o) => o,
        Err(e) => {
            warn!("LLM inference failed: {}", e);
            return Ok((0, 0));
        }
    };

    info!(
        "LLM returned {} projects, {} relations",
        output.projects.len(),
        output.relations.len()
    );

    // 3. Collect all file IDs by path for quick lookup
    let all_files = sqlite.all_paths().unwrap_or_default();
    let mut path_to_id: HashMap<String, i64> = HashMap::new();
    for (id, path, _) in &all_files {
        path_to_id.insert(path.clone(), *id);
    }

    // 4. Write projects and link files
    let mut project_name_to_id: HashMap<String, i64> = HashMap::new();
    let mut projects_created = 0;

    for proj in &output.projects {
        let root_dirs_json = serde_json::to_string(&proj.dirs).unwrap_or_default();
        let id = project_store.insert(
            &proj.name,
            proj.assay_type.as_deref(),
            proj.species.as_deref(),
            proj.species_confidence.as_deref(),
            Some(&root_dirs_json),
            proj.summary.as_deref(),
        )?;
        project_name_to_id.insert(proj.name.clone(), id);
        projects_created += 1;

        // Link files under project dirs to this project
        for dir in &proj.dirs {
            for (file_id, file_path, _) in &all_files {
                if file_path.starts_with(dir) {
                    project_store.link_file(id, *file_id).ok();
                }
            }
        }
    }

    // 5. Write project relations
    for rel in &output.relations {
        if let (Some(&a_id), Some(&b_id)) = (
            project_name_to_id.get(&rel.project_a),
            project_name_to_id.get(&rel.project_b),
        ) {
            project_store
                .add_relation(a_id, b_id, &rel.relation, rel.score, None)
                .ok();
        }
    }

    // 7. Back-sync LLM metadata to file-level bio_metadata
    let mut files_updated = 0;
    for proj in &output.projects {
        if let Some(&_proj_id) = project_name_to_id.get(&proj.name) {
            let assay_val = proj.assay_type.clone().unwrap_or_default();
            let species_val = proj.species.clone().unwrap_or_default();
            for dir in &proj.dirs {
                for (file_id, file_path, _) in &all_files {
                    if file_path.starts_with(dir) {
                        if let Ok(Some(entry)) = sqlite.get_by_id(*file_id) {
                            let mut meta = entry.bio_metadata.unwrap_or_default();
                            if !assay_val.is_empty() {
                                meta.assay_type = Some(assay_val.clone());
                            }
                            if !species_val.is_empty() {
                                meta.species = Some(species_val.clone());
                            }
                            meta.project = Some(proj.name.clone());
                            if let Err(e) = sqlite.update_bio_metadata(*file_id, &meta) {
                                warn!("Failed to back-sync metadata for {}: {}", file_path, e);
                            } else {
                                files_updated += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    info!("Back-synced LLM metadata to {} files", files_updated);

    // 8. Generate pending review items for low/medium confidence projects
    let mut pending_items: Vec<crate::review::PendingItem> = Vec::new();
    for proj in &output.projects {
        let needs_review = proj.species_confidence.as_deref() == Some("low")
            || proj.species_confidence.as_deref() == Some("medium");
        if needs_review {
            let candidates = generate_species_candidates(llm_client, proj);
            pending_items.push(crate::review::PendingItem {
                project: proj.name.clone(),
                field: "species".into(),
                current_value: proj.species.clone(),
                confidence: proj.species_confidence.clone(),
                candidates,
                timestamp: crate::review::ReviewStore::now(),
            });
        }
    }
    if !pending_items.is_empty() {
        let store = crate::review::ReviewStore::new();
        store.save(&pending_items)?;
        info!("Saved {} pending review items", pending_items.len());
    }

    Ok((projects_created, output.relations.len()))
}

/// Collect directory tree summary from indexed files.
/// Returns (path, file_count, sample_filenames) sorted by file count desc.
fn collect_directory_summary(
    sqlite: &crate::index::sqlite::SqliteStore,
) -> Vec<(String, usize, Vec<String>)> {
    let all = sqlite.all_paths().unwrap_or_default();
    let mut dir_map: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for (_, path, _) in &all {
        if let Some(parent) = std::path::Path::new(path).parent() {
            let dir_path = parent.to_string_lossy().to_string();
            let entry = dir_map.entry(dir_path).or_insert((0, Vec::new()));
            entry.0 += 1;
            if let Some(name) = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
            {
                if entry.1.len() < 8 {
                    entry.1.push(name.to_string());
                }
            }
        }
    }

    let mut result: Vec<_> = dir_map.into_iter().collect();
    result.sort_by(|a, b| b.1 .0.cmp(&a.1 .0)); // Sort by file count desc
    result
        .into_iter()
        .map(|(path, (count, files))| (path, count, files))
        .collect()
}

fn generate_species_candidates(
    llm_client: &LlmClient,
    proj: &crate::llm::prompt::LlmProject,
) -> Vec<String> {
    if !llm_client.is_configured() {
        return vec![];
    }
    let prompt = format!(
        "数据项目 '{}' 的物种未知。根据项目名和上下文，列出最可能的4个物种名，以逗号分隔，只返回物种名。\n项目名: {}\n描述: {}",
        proj.name,
        proj.name,
        proj.summary.as_deref().unwrap_or("")
    );
    match llm_client.infer_candidates(&prompt) {
        Ok(candidates) => candidates,
        Err(e) => {
            warn!("Failed to generate candidates for {}: {}", proj.name, e);
            vec![]
        }
    }
}
