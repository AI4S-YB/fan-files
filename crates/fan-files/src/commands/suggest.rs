use fan_core::config::Config;
use fan_core::suggest::SuggestEngine;

pub fn run(config: &Config, path: &str, json: bool) {
    let index = match fan_core::index::open_index(config, fan_core::index::IndexMode::ReadOnly) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    let suggestions = SuggestEngine::suggest(&index, path, 10).unwrap_or_default();

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
