use fan_core::config::{Config, DataLayer};
use fan_core::infer;
use fan_core::infer_hierarchical;
use fan_core::index::sqlite::SqliteStore;
use fan_core::llm::LlmClient;
use fan_core::project::ProjectStore;
use std::sync::Arc;

pub fn run(config: &Config, layer: &DataLayer) {
    run_inner(config, layer, true); // hierarchical by default
}

pub fn run_flat(config: &Config, layer: &DataLayer) {
    run_inner(config, layer, false);
}

fn run_inner(config: &Config, layer: &DataLayer, hierarchical: bool) {
    let llm_client = LlmClient::new(config.llm.clone());
    if !llm_client.is_configured() {
        eprintln!("LLM not configured.");
        eprintln!("Add [llm] section to ~/.fan-files/config.toml:");
        eprintln!("  [llm]");
        eprintln!("  endpoint = \"https://api.openai.com/v1/chat/completions\"");
        eprintln!("  api_key = \"sk-...\"");
        eprintln!("  model = \"gpt-4o-mini\"");
        return;
    }

    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    let sqlite = match SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    let project_store = ProjectStore::new(Arc::clone(&sqlite.conn));
    // Resolve scan root: prefer servers config, then scan.include, then "/"
    let servers = config.enabled_servers();
    let scan_root = servers.first()
        .and_then(|(_, cfg)| cfg.scan_roots.first().map(|s| s.as_str()))
        .or_else(|| config.scan.include.first().map(|s| s.as_str()))
        .unwrap_or("/");

    println!("Running LLM inference on indexed files...");

    let result = if hierarchical {
        infer_hierarchical::run_hierarchical_inference(&sqlite, &project_store, &llm_client, scan_root)
    } else {
        infer::run_inference(&sqlite, &project_store, &llm_client, scan_root)
    };

    match result {
        Ok((projects, relations)) => {
            println!("Inference complete: {} projects, {} relations", projects, relations);
            if let Ok(all_projects) = project_store.all() {
                for p in &all_projects {
                    println!();
                    println!("  Project: {}", p.name);
                    if let Some(ref at) = p.assay_type {
                        println!("    Assay:   {}", at);
                    }
                    if let Some(ref sp) = p.species {
                        println!(
                            "    Species: {} ({}, source: {})",
                            sp,
                            p.species_confidence.as_deref().unwrap_or("?"),
                            p.species_source.as_deref().unwrap_or("llm")
                        );
                    }
                    if let Some(ref s) = p.summary {
                        println!("    Summary: {}", s);
                    }
                }
            }
        }
        Err(e) => eprintln!("Inference failed: {}", e),
    }
}
