use fan_core::config::{Config, DataLayer};
use std::path::Path;

pub fn run(config: &Config, layer: &DataLayer, path: &str, json: bool) {
    let index = match fan_core::index::open_index_for_layer(config, layer, fan_core::index::IndexMode::ReadOnly) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    match index.sqlite.get_by_path(Path::new(path)) {
        Ok(Some(entry)) => {
            let ts_to_str = |ts: i64| -> String {
                std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(ts as u64))
                    .map(|t| format!("{:?}", t))
                    .unwrap_or_else(|| ts.to_string())
            };

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": entry.path.to_string_lossy(),
                        "source_server": entry.source_server,
                        "size": entry.size,
                        "size_mb": format!("{:.2}", entry.size as f64 / 1_048_576.0),
                        "mtime": ts_to_str(entry.mtime_secs),
                        "mime": entry.mime_type,
                        "format": entry.format_info,
                        "bio_metadata": entry.bio_metadata,
                        "indexed_at": ts_to_str(entry.indexed_at),
                        "updated_at": ts_to_str(entry.updated_at),
                    }))
                    .unwrap()
                );
            } else {
                println!("Path:       {}", entry.path.display());
                if entry.source_server != "local" {
                    let label = config.servers.servers
                        .get(&entry.source_server)
                        .and_then(|c| c.label.as_ref())
                        .map(|l| format!(" ({})", l))
                        .unwrap_or_default();
                    println!("Source:     {}{}", entry.source_server, label);
                }
                println!(
                    "Size:       {:.2} MB ({} bytes)",
                    entry.size as f64 / 1_048_576.0,
                    entry.size
                );
                println!("MIME:       {}", entry.mime_type.as_deref().unwrap_or("-"));
                println!("Format:     {:?}", entry.format_info);
                if let Some(ref meta) = entry.bio_metadata {
                    println!("Bio Metadata:");
                    if let Some(at) = &meta.assay_type {
                        println!("  Assay:   {}", at);
                    }
                    if let Some(sp) = &meta.species {
                        println!("  Species: {}", sp);
                    }
                    if let Some(ti) = &meta.tissue {
                        println!("  Tissue:  {}", ti);
                    }
                    if let Some(gb) = &meta.genome_build {
                        println!("  Genome:  {}", gb);
                    }
                    if let Some(pr) = &meta.project {
                        println!("  Project: {}", pr);
                    }
                    if !meta.tags.is_empty() {
                        println!("  Tags:    {:?}", meta.tags);
                    }
                }
                println!("Indexed:    {}", entry.indexed_at);
            }
        }
        Ok(None) => {
            if json {
                println!("{{\"error\": \"file not in index: {}\"}}", path);
            } else {
                eprintln!("File not in index: {}", path);
                eprintln!("Run 'fan-files daemon' to scan and index files.");
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
