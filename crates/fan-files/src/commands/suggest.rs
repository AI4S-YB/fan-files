use fan_core::config::{Config, DataLayer};
use fan_core::suggest::SuggestEngine;

pub fn run(_config: &Config, layer: &DataLayer, path: &str, json: bool) {
    let sqlite = match fan_core::index::open_sqlite_read_only(layer) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    let suggestions = SuggestEngine::suggest(&sqlite, path, 10).unwrap_or_default();

    if json {
        println!("{}", serde_json::to_string_pretty(&suggestions).unwrap());
    } else {
        println!("Suggestions for {}:", path);
        for s in &suggestions {
            println!(
                "  {:.3}  {}  {}  {}",
                s.score,
                s.path,
                s.assay_type.as_deref().unwrap_or("-"),
                s.summary
            );
        }
        if suggestions.is_empty() {
            println!("  No related data found. Try indexing first with 'fan-files daemon'.");
        }
    }
}
