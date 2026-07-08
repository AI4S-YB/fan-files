//! `fan-files discover` — Progressive Discovery pipeline.
//!
//! Phase A: Lightweight directory walk → LLM → scan_targets
//! Phase B: Targeted file scan (only valuable dirs)
//! Phase C: Hierarchical LLM inference

use fan_core::config::{Config, DataLayer};
use fan_core::discovery;
use fan_core::infer_hierarchical;
use fan_core::index::IndexEngine;
use fan_core::llm::LlmClient;
use fan_core::project::ProjectStore;
use std::sync::Arc;

pub fn run(config: &Config, layer: &DataLayer) {
    run_inner(config, layer, false);
}

pub fn run_deep(config: &Config, layer: &DataLayer) {
    run_inner(config, layer, true);
}

fn run_inner(config: &Config, layer: &DataLayer, deep: bool) {
    let llm_client = LlmClient::new(config.llm.clone());
    if !llm_client.is_configured() {
        eprintln!("LLM not configured. Set [llm] in config.toml.");
        return;
    }

    // Resolve scan roots
    let servers = config.enabled_servers();
    let scan_roots: Vec<&str> = servers.iter()
        .flat_map(|(_, cfg)| cfg.scan_roots.iter().map(|s| s.as_str()))
        .collect();
    if scan_roots.is_empty() {
        eprintln!("No scan roots configured. Use 'fan-files servers add' first.");
        return;
    }

    println!("╔══════════════════════════════════════════╗");
    println!("║   Progressive Discovery Pipeline         ║");
    println!("╚══════════════════════════════════════════╝");
    println!();

    // ═══ Phase A: Lightweight walk + LLM pre-filter ═══
    let mode_label = if deep { "Recursive (3→5→7)" } else { "Shallow (depth 3)" };
    println!("═══ Phase A: Directory Analysis ({}) ═══", mode_label);
    let mut all_targets: Vec<String> = Vec::new();
    let mut total_skipped = 0;
    // TODO: collect Phase A LLM species/assay hints and pass to Phase C
    let _phase_a_hints: Vec<(String, String, String)> = Vec::new();

    for root in &scan_roots {
        eprintln!("  Analyzing directory structure: {}", root);
        let result = if deep {
            discovery::run_recursive_phase_a(root, &llm_client, 9)
        } else {
            discovery::run_phase_a(root, &llm_client)
        };

        // If Phase A fails, retry once with shallow mode before falling back
        let result = match result {
            Err(e) => {
                eprintln!("  Phase A failed: {}. Retrying with shallow mode...", e);
                discovery::run_phase_a(root, &llm_client)
            }
            ok => ok,
        };

        match result {
            Ok((targets, skips)) => {
                eprintln!("  → {} dirs to scan, {} skipped", targets.len(), skips.len());
                total_skipped += skips.len();
                for t in targets {
                    let abs = if t.starts_with('/') {
                        t.clone()
                    } else {
                        format!("{}/{}", root.trim_end_matches('/'), t.trim_start_matches('/'))
                    };
                    all_targets.push(abs);
                    // Collect hints from Phase A for Phase C
                    // (Phase A LLM may return species/assay alongside targets)
                }
            }
            Err(e) => {
                eprintln!("  Phase A failed after retry: {}. Scanning root as-is.", e);
                all_targets.push(root.to_string());
            }
        }
    }

    if all_targets.is_empty() {
        eprintln!("No directories to scan. Check your config or LLM.");
        return;
    }

    eprintln!(
        "  Phase A complete: {} targets, {} skipped",
        all_targets.len(), total_skipped
    );
    println!();

    // ═══ Phase B: Targeted scan (only valuable dirs) ═══
    println!("═══ Phase B: Targeted Scan ═══");
    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };

    // Open index engine (SQLite + Tantivy)
    let index = match IndexEngine::open_at(&data_dir, config, false) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    // Scan each target directory with batch transactions
    let mut total_files = 0u64;
    let mut batch_count = 0usize;
    index.sqlite.begin_batch().ok();

    for target in &all_targets {
        let scanner = fan_core::scanner::Scanner::new(
            vec![target.clone()],
            config.scan.exclude.clone(),
            "discovery".to_string(),
        );

        eprintln!("  Scanning: {}", target);
        for file_info in scanner.scan() {
            match index.index_file(&file_info, None) {
                Ok(_) => {
                    total_files += 1;
                    batch_count += 1;
                }
                Err(e) => eprintln!("  Failed to index {}: {}", file_info.path.display(), e),
            }
            if batch_count >= 1000 {
                index.sqlite.commit_batch().ok();
                index.tantivy.commit().ok();
                batch_count = 0;
                index.sqlite.begin_batch().ok();
            }
        }
    }
    if batch_count > 0 {
        index.sqlite.commit_batch().ok();
        index.tantivy.commit().ok();
    }
    eprintln!("  Phase B complete: {} files indexed", total_files);
    println!();

    // ═══ Phase C: Hierarchical inference ═══
    println!("═══ Phase C: LLM Inference ═══");
    let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));

    for root in &scan_roots {
        eprintln!("  Inferring: {}", root);
        match infer_hierarchical::run_hierarchical_inference(&index.sqlite, &project_store, &llm_client, root) {
            Ok((projects, _)) => eprintln!("  → {} projects", projects),
            Err(e) => eprintln!("  Inference failed for {}: {}", root, e),
        }
    }

    println!();
    println!("╔══════════════════════════════════════════╗");
    println!("║   Discovery Complete                     ║");
    println!("║   {} files indexed, {} targets scanned ║", total_files, all_targets.len());
    println!("╚══════════════════════════════════════════╝");
}
