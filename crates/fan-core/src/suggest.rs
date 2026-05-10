use crate::index::IndexEngine;
use fan_plugin_sdk::{BioMetadata, DataSource, SearchResult};
use tracing::info;

/// 实验类型互补矩阵
static COMPLEMENTARY_ASSAYS: &[(&str, &[&str])] = &[
    ("RNA-seq", &["ChIP-seq", "ATAC-seq", "WGBS"] as &[&str]),
    ("WGS", &["WGBS", "RNA-seq", "ChIP-seq"]),
    ("scRNA-seq", &["scATAC-seq", "CITE-seq"]),
    ("ChIP-seq", &["RNA-seq", "ATAC-seq"]),
    ("ATAC-seq", &["RNA-seq", "ChIP-seq"]),
    ("WGBS", &["RNA-seq", "WGS"]),
];

pub struct SuggestEngine;

impl SuggestEngine {
    pub fn suggest(
        index: &IndexEngine,
        project_dir: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
        // 1. Search for files in the project directory via Tantivy
        let project_files = index.tantivy.search(project_dir, 50).unwrap_or_default();
        info!("Found {} files in project directory", project_files.len());

        // 2. Collect bio metadata from project files
        let mut project_meta: Vec<BioMetadata> = Vec::new();
        for (file_id, _score) in &project_files {
            if let Ok(Some(entry)) = index.sqlite.get_by_id(*file_id) {
                if let Some(meta) = &entry.bio_metadata {
                    project_meta.push(meta.clone());
                }
            }
        }

        if project_meta.is_empty() {
            info!("No bio metadata found for project, returning empty suggestions");
            return Ok(vec![]);
        }

        // 3. Extract key dimensions from project metadata
        let species = project_meta.iter().find_map(|m| m.species.clone());
        let tissue = project_meta.iter().find_map(|m| m.tissue.clone());
        let project = project_meta.iter().find_map(|m| m.project.clone());
        let assay_types: Vec<String> = project_meta.iter()
            .filter_map(|m| m.assay_type.clone())
            .collect();

        // 4. Find complementary assay types
        let want_assays: Vec<String> = assay_types.iter()
            .flat_map(|a| {
                COMPLEMENTARY_ASSAYS.iter()
                    .filter(|(k, _)| k == &a.as_str())
                    .flat_map(|(_, v)| v.iter().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect();

        info!("Want complementary assays: {:?}", want_assays);

        // 5. Score all indexed files
        let candidates = index.sqlite.all_paths().unwrap_or_default();
        let project_file_ids: Vec<i64> = project_files.iter().map(|(id, _)| *id).collect();
        let mut scored: Vec<SearchResult> = Vec::new();

        for (id, path_str, _mtime) in &candidates {
            // Skip files already in the project
            if project_file_ids.contains(id) {
                continue;
            }

            if let Ok(Some(entry)) = index.sqlite.get_by_id(*id) {
                let mut score: f64 = 0.0;
                let mut reasons: Vec<String> = Vec::new();

                if let Some(ref meta) = entry.bio_metadata {
                    if species.is_some() && meta.species == species {
                        score += 0.3;
                        reasons.push("same species".into());
                    }
                    if tissue.is_some() && meta.tissue == tissue {
                        score += 0.2;
                        reasons.push("same tissue".into());
                    }
                    if project.is_some() && meta.project == project {
                        score += 0.3;
                        reasons.push("same project".into());
                    }
                    if meta.assay_type.as_ref().map(|a| want_assays.contains(a)).unwrap_or(false) {
                        score += 0.15;
                        reasons.push("complementary assay".into());
                    }
                    if meta.genome_build.is_some() {
                        score += 0.05;
                    }
                }

                if score > 0.1 {
                    scored.push(SearchResult {
                        path: path_str.clone(),
                        score,
                        file_type: entry.format_info.as_ref().map(|f| f.file_type.clone()),
                        assay_type: entry.bio_metadata.as_ref().and_then(|m| m.assay_type.clone()),
                        species: entry.bio_metadata.as_ref().and_then(|m| m.species.clone()),
                        tags: entry.bio_metadata.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
                        summary: reasons.join(", "),
                        source: DataSource::Local,
                    });
                }
            }
        }

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        info!("Returning {} suggestions", scored.len());
        Ok(scored)
    }
}
