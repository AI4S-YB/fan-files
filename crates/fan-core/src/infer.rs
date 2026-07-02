use crate::llm::LlmClient;
use crate::project::ProjectStore;
use std::collections::HashMap;
use tracing::{info, warn};

const BATCH_SIZE: usize = 80; // dirs per LLM call (smaller batches avoid JSON truncation)

/// Run the full LLM inference pipeline (batched, with progress).
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

    // 1. Collect bio-relevant directories only (filter noise)
    let dirs = collect_directory_summary(sqlite);
    if dirs.is_empty() {
        info!("No bio-relevant directories found for inference");
        return Ok((0, 0));
    }

    let total_dirs = dirs.len();
    let batches = (total_dirs + BATCH_SIZE - 1) / BATCH_SIZE;
    eprintln!(
        "  LLM inference: {} bio-relevant dirs → {} batches ({} dirs/batch)",
        total_dirs, batches, BATCH_SIZE
    );

    // 2. Collect all file IDs for linking
    let all_files = sqlite.all_paths().unwrap_or_default();
    let mut path_to_id: HashMap<String, i64> = HashMap::new();
    for (id, path, _) in &all_files {
        path_to_id.insert(path.clone(), *id);
    }

    let mut all_projects: Vec<crate::llm::prompt::LlmProject> = Vec::new();
    let mut all_relations: Vec<crate::llm::prompt::LlmRelation> = Vec::new();
    let mut seen_project_names: HashMap<String, usize> = HashMap::new();

    // 3. Process each batch
    for batch_idx in 0..batches {
        let start = batch_idx * BATCH_SIZE;
        let end = std::cmp::min(start + BATCH_SIZE, total_dirs);
        let batch_dirs = &dirs[start..end];

        eprintln!(
            "  batch {}/{} ({} dirs)...",
            batch_idx + 1, batches, batch_dirs.len()
        );

        let summary = crate::llm::prompt::build_directory_summary(
            &format!("{} (batch {}/{})", scan_root, batch_idx + 1, batches),
            batch_dirs,
        );

        match llm_client.infer_projects(&summary) {
            Ok(output) => {
                let proj_count = output.projects.len();
                let rel_count = output.relations.len();
                // Merge projects (dedup by name: merge dirs on collision)
                // Restore absolute paths from relative (LLM returns rel paths)
                let abs_prefix = if scan_root.ends_with('/') {
                    scan_root.to_string()
                } else {
                    format!("{}/", scan_root)
                };
                for mut proj in output.projects {
                    // Restore absolute paths
                    proj.dirs = proj.dirs.into_iter().map(|d| {
                        if d.starts_with('/') { d }
                        else { format!("{}{}", abs_prefix, d.trim_start_matches('/')) }
                    }).collect();
                    if let Some(&existing_idx) = seen_project_names.get(&proj.name) {
                        if existing_idx < all_projects.len() {
                            all_projects[existing_idx].dirs.extend(proj.dirs);
                        }
                    } else {
                        seen_project_names.insert(proj.name.clone(), all_projects.len());
                        all_projects.push(proj);
                    }
                }
                all_relations.extend(output.relations);
                eprintln!(
                    "    → {} projects, {} relations",
                    proj_count, rel_count
                );
            }
            Err(e) => {
                warn!("  batch {} failed: {}", batch_idx + 1, e);
                eprintln!("  ⚠ batch {} failed, continuing...", batch_idx + 1);
            }
        }
    }

    if all_projects.is_empty() {
        eprintln!("  No projects inferred.");
        return Ok((0, 0));
    }

    // Post-processing: dedup projects with same species + common name prefix
    let before_dedup = all_projects.len();
    all_projects = dedup_projects(all_projects);
    if before_dedup != all_projects.len() {
        eprintln!("  Dedup: {} → {} projects", before_dedup, all_projects.len());
    }
    eprintln!("  Total: {} projects", all_projects.len());

    // 4. Write projects and link files
    let mut project_name_to_id: HashMap<String, i64> = HashMap::new();
    let mut projects_created = 0;

    for proj in &all_projects {
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

        // Link files under project dirs
        for dir in &proj.dirs {
            for (file_id, file_path, _) in &all_files {
                if file_path.starts_with(dir) {
                    project_store.link_file(id, *file_id).ok();
                }
            }
        }
    }

    // 5. Write relations
    for rel in &all_relations {
        if let (Some(&a_id), Some(&b_id)) = (
            project_name_to_id.get(&rel.project_a),
            project_name_to_id.get(&rel.project_b),
        ) {
            project_store.add_relation(a_id, b_id, &rel.relation, rel.score, None).ok();
        }
    }

    // 6. Back-sync metadata to files
    let mut files_updated = 0;
    for proj in &all_projects {
        if let Some(&_proj_id) = project_name_to_id.get(&proj.name) {
            let assay_val = proj.assay_type.clone().unwrap_or_default();
            let species_val = proj.species.clone().unwrap_or_default();
            let mut batch_updated = 0;
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
                                batch_updated += 1;
                            }
                        }
                    }
                }
            }
            if batch_updated > 0 {
                info!(
                    "  {} → {} files ({:?}, {:?})",
                    proj.name, batch_updated, proj.assay_type, proj.species
                );
            }
        }
    }
    info!("Back-synced LLM metadata to {} files", files_updated);

    // 7. Generate pending review items for low/medium confidence
    let mut pending_items: Vec<crate::review::PendingItem> = Vec::new();
    for proj in &all_projects {
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

    eprintln!(
        "  ✅ Done: {} projects, {} files tagged, {} relations",
        projects_created, files_updated, all_relations.len()
    );

    Ok((projects_created, all_relations.len()))
}

/// Check if a filename indicates a bioinformatics-relevant file.
fn is_bio_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".fastq.gz") || lower.ends_with(".fq.gz") || lower.ends_with(".fastq") ||
    lower.ends_with(".fasta") || lower.ends_with(".fa.gz") || lower.ends_with(".fa") ||
    lower.ends_with(".fna") || lower.ends_with(".faa") || lower.ends_with(".ffn") ||
    lower.ends_with(".bam") || lower.ends_with(".vcf.gz") || lower.ends_with(".vcf") ||
    lower.ends_with(".gff3") || lower.ends_with(".gtf") || lower.ends_with(".bed") ||
    lower.ends_with(".h5") || lower.ends_with(".hdf5") ||
    lower.ends_with(".gff") || lower.ends_with(".gff.gz") ||
    lower.ends_with(".sam") || lower.ends_with(".cram") ||
    lower.ends_with(".bw") || lower.ends_with(".bigwig") ||
    lower.ends_with(".tsv.gz") || lower.ends_with(".csv.gz") ||
    lower.ends_with(".tsv") || lower.ends_with(".csv") ||
    lower.ends_with(".txt.gz") || lower.ends_with(".tab.gz") ||
    lower.ends_with(".narrowpeak") || lower.ends_with(".broadpeak") ||
    lower.ends_with(".maf") || lower.ends_with(".phylip") ||
    lower.ends_with(".nex") || lower.ends_with(".nexus") ||
    lower.ends_with(".gfa") || lower.ends_with(".agp") ||
    lower.ends_with(".chain") || lower.ends_with(".net") ||
    lower.ends_with(".2bit") || lower.ends_with(".sizes") ||
    lower.ends_with(".idx") || lower.ends_with(".tbi") || lower.ends_with(".csi")
}

/// Collect directory tree summary from indexed files (bio-relevant only, by default).
/// Returns (path, file_count, sample_filenames) sorted by file count desc.
fn collect_directory_summary(
    sqlite: &crate::index::sqlite::SqliteStore,
) -> Vec<(String, usize, Vec<String>)> {
    let all = sqlite.all_paths().unwrap_or_default();
    let mut dir_map: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for (_, file_path, _) in &all {
        let path = std::path::Path::new(file_path);
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let parent = path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();

        // Only count directories that contain bio-relevant files
        // (but we still include ALL siblings — non-bio files help context)
        let entry = dir_map.entry(parent.clone()).or_insert((0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 8 {
            let display = if fname.chars().count() > 60 {
                let truncated: String = fname.chars().take(57).collect();
                format!("{}...", truncated)
            } else {
                fname.to_string()
            };
            if !entry.1.contains(&display) {
                entry.1.push(display);
            }
        }
    }

    // Filter to only directories that have at least 1 bio-relevant file
    let bio_parents: std::collections::HashSet<String> = all.iter()
        .filter(|(_, p, _)| is_bio_file(std::path::Path::new(p)
            .file_name().and_then(|n| n.to_str()).unwrap_or("")))
        .map(|(_, p, _)| {
            std::path::Path::new(p).parent()
                .map(|par| par.to_string_lossy().to_string())
                .unwrap_or_default()
        })
        .collect();

    let mut result: Vec<_> = dir_map.into_iter()
        .filter(|(dir, _)| bio_parents.contains(dir))
        .collect();

    result.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    result.into_iter()
        .map(|(path, (count, files))| (path, count, files))
        .collect()
}

/// Merge projects that likely represent the same dataset (same species, common name prefix).
fn dedup_projects(mut projects: Vec<crate::llm::prompt::LlmProject>) -> Vec<crate::llm::prompt::LlmProject> {
    let mut merged = true;
    while merged {
        merged = false;
        let mut result: Vec<crate::llm::prompt::LlmProject> = Vec::new();
        let mut skip = vec![false; projects.len()];
        for i in 0..projects.len() {
            if skip[i] { continue; }
            let mut base = projects[i].clone();
            for j in (i + 1)..projects.len() {
                if skip[j] { continue; }
                let same_species = base.species.as_deref() == projects[j].species.as_deref()
                    && base.species.is_some();
                let shared_prefix = common_prefix_len(&base.name, &projects[j].name) >= 4;
                if same_species && shared_prefix {
                    base.dirs.extend(projects[j].dirs.clone());
                    // Merge summaries
                    if let Some(ref s) = projects[j].summary {
                        base.summary = Some(match base.summary {
                            Some(ref existing) => format!("{}; {}", existing, s),
                            None => s.clone(),
                        });
                    }
                    skip[j] = true;
                    merged = true;
                }
            }
            result.push(base);
        }
        projects = result;
    }
    projects
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(ac, bc)| ac == bc).count()
}

fn generate_species_candidates(
    llm_client: &LlmClient,
    proj: &crate::llm::prompt::LlmProject,
) -> Vec<String> {
    if !llm_client.is_configured() {
        return vec![];
    }
    let prompt = format!(
        "List 4 most likely species names for this project, comma-separated, Latin binomial only. No explanation, no Chinese, no prefixes.\nProject: {}\nDescription: {}",
        proj.name,
        proj.summary.as_deref().unwrap_or("")
    );
    let raw = match llm_client.infer_candidates(&prompt) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to generate candidates for {}: {}", proj.name, e);
            return vec![];
        }
    };
    // Clean: strip Chinese text, explanations, and non-species prefixes
    raw.into_iter()
        .filter_map(|s| {
            let cleaned = s
                // Remove Chinese characters and common prefixes
                .chars()
                .skip_while(|c| c.is_whitespace() || *c == '-' || *c == ':' || *c == '：')
                .collect::<String>();
            let cleaned = cleaned
                .split('\n')
                .last()  // Take last line if multi-line
                .unwrap_or(&cleaned)
                .trim()
                .to_string();
            // Keep only if it looks like a Latin binomial (two words, no Chinese)
            let has_chinese = cleaned.chars().any(|c| c as u32 > 0x4E00);
            let word_count = cleaned.split_whitespace().count();
            if !cleaned.is_empty() && !has_chinese && word_count >= 2 && word_count <= 3 {
                Some(cleaned)
            } else {
                None
            }
        })
        .collect()
}
