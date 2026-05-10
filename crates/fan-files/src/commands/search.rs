use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_plugin_sdk::{DataSource, SearchResult};

pub fn run(config: &Config, query: &str, json: bool) {
    let index = match IndexEngine::open(config) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    // Search Tantivy for keyword matches
    let tantivy_results = index.tantivy.search(query, 20).unwrap_or_default();

    let mut results: Vec<SearchResult> = Vec::new();
    for (file_id, score) in &tantivy_results {
        if let Ok(Some(entry)) = index.sqlite.get_by_id(*file_id) {
            results.push(SearchResult {
                path: entry.path.to_string_lossy().to_string(),
                score: *score as f64,
                file_type: entry.format_info.as_ref().map(|f| f.file_type.clone()),
                assay_type: entry
                    .bio_metadata
                    .as_ref()
                    .and_then(|m| m.assay_type.clone()),
                species: entry
                    .bio_metadata
                    .as_ref()
                    .and_then(|m| m.species.clone()),
                tags: entry
                    .bio_metadata
                    .as_ref()
                    .map(|m| m.tags.clone())
                    .unwrap_or_default(),
                summary: entry
                    .bio_metadata
                    .as_ref()
                    .map(|m| {
                        let parts: Vec<String> = vec![
                            m.assay_type.clone(),
                            m.species.clone(),
                            m.tissue.clone(),
                            m.project.clone(),
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        parts.join(", ")
                    })
                    .unwrap_or_default(),
                source: DataSource::Local,
            });
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        for r in &results {
            println!(
                "{:.3}  {}  {:?}  {:?}  {}",
                r.score, r.path, r.assay_type, r.species, r.summary
            );
        }
        if results.is_empty() {
            println!("No results found for: {}", query);
        }
    }
}
