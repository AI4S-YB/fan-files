use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_core::infer;
use fan_core::llm::LlmClient;
use fan_core::project::ProjectStore;
use std::sync::Arc;

pub fn run(config: &Config) {
    let index = match IndexEngine::open(config, true) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

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

    let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));

    let scan_root = config
        .scan
        .include
        .first()
        .map(|s| s.as_str())
        .unwrap_or("/");

    println!("Running LLM inference on indexed files...");
    match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root, config.llm.bold_enabled) {
        Ok((projects, relations)) => {
            println!(
                "Inference complete: {} projects, {} relations",
                projects, relations
            );

            // Display results
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
