use fan_core::config::{Config, DataLayer};
use fan_core::infer_hierarchical;
use fan_core::index;
use fan_core::llm::LlmClient;

pub fn run(config: &Config, layer: &DataLayer) {
    let llm_client = LlmClient::new(config.llm.clone());
    if !llm_client.is_configured() {
        eprintln!("LLM not configured. Add [llm] to ~/.fan-files/config.toml");
        return;
    }

    let sqlite = match index::open_sqlite(layer) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };

    let servers = config.enabled_servers();
    let scan_root = servers.first()
        .and_then(|(_, cfg)| cfg.scan_roots.first().map(|s| s.as_str()))
        .or_else(|| config.scan.include.first().map(|s| s.as_str()))
        .unwrap_or("/");

    println!("Running Phase C LLM inference...");
    match infer_hierarchical::run_dataset_asset_inference(&sqlite, &llm_client, config.threads, scan_root, &[]) {
        Ok(n) => println!("Inference complete: {} datasets created", n),
        Err(e) => eprintln!("Inference failed: {}", e),
    }
}

/// [deprecated] Flat mode — now uses same Phase C pipeline
pub fn run_flat(config: &Config, layer: &DataLayer) {
    run(config, layer);
}
