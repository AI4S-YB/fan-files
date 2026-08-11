use fan_core::config::{Config, DataLayer};
use fan_core::index::{self, IndexMode};

pub fn run(config: &Config, layer: &DataLayer) {
    let index = match index::open_index_for_layer(config, layer, IndexMode::ReadWrite) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    let files = match index.sqlite.all_paths() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to read files: {}", e);
            return;
        }
    };

    let total = files.len();
    eprintln!("Rebuilding Tantivy index for {} files...", total);

    let mut enriched = 0u64;
    for (id, path_str, _) in &files {
        let meta_text = match index.sqlite.get_by_id(*id) {
            Ok(Some(entry)) => {
                let species = entry.bio_metadata.as_ref()
                    .and_then(|m| m.species.as_deref()).unwrap_or("");
                let assay = entry.bio_metadata.as_ref()
                    .and_then(|m| m.assay_type.as_deref()).unwrap_or("");
                let fmt = entry.format_info.as_ref()
                    .map(|f| format!("{:?}", f)).unwrap_or_default();
                if species.is_empty() && assay.is_empty() {
                    format!("{} {}", path_str, fmt)
                } else {
                    format!("{} {} {} {}", path_str, species, assay, fmt)
                }
            }
            _ => path_str.clone(),
        };
        if index.tantivy.index_file(*id, &std::path::Path::new(&path_str), &meta_text, &[]).is_ok() {
            enriched += 1;
        }
        if enriched % 100_000 == 0 {
            eprintln!("  Progress: {}/{} files", enriched, total);
        }
    }

    index.tantivy.commit().ok();
    eprintln!("Tantivy index rebuilt: {} files enriched", enriched);
}
