use fan_core::config::Config;
use fan_core::index::IndexEngine;

pub fn run(config: &Config, category: Option<&str>, tag: Option<&str>, json: bool) {
    let index = match IndexEngine::open(config) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    let entries = if let Some(tag) = tag {
        index.sqlite.list_by_tag(tag, 100).unwrap_or_default()
    } else if let Some(cat) = category {
        // Use Tantivy to search for category keyword
        let results = index.tantivy.search(cat, 100).unwrap_or_default();
        results
            .iter()
            .filter_map(|(id, _)| index.sqlite.get_by_id(*id).ok().flatten())
            .collect()
    } else {
        // No filter: list all file paths
        let all = index.sqlite.all_paths().unwrap_or_default();
        all.iter()
            .filter_map(|(id, _, _)| index.sqlite.get_by_id(*id).ok().flatten())
            .take(100)
            .collect()
    };

    if json {
        let output: Vec<_> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "path": e.path.to_string_lossy(),
                    "size": e.size,
                    "type": e.format_info.as_ref().map(|f| &f.file_type),
                    "assay": e.bio_metadata.as_ref().and_then(|m| m.assay_type.as_ref()),
                    "species": e.bio_metadata.as_ref().and_then(|m| m.species.as_ref()),
                    "tags": e.bio_metadata.as_ref().map(|m| &m.tags),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        for e in &entries {
            let assay = e
                .bio_metadata
                .as_ref()
                .and_then(|m| m.assay_type.as_ref())
                .map(|s| s.as_str())
                .unwrap_or("-");
            let size_mb = e.size as f64 / 1_048_576.0;
            println!("{:>8.1} MB  [{}]  {}", size_mb, assay, e.path.display());
        }
    }
}
