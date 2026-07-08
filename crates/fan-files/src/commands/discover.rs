//! `fan-files discover` — Progressive Discovery pipeline.
//!
//! Phase A: Lightweight directory walk → LLM → scan_targets
//! Phase B: Targeted file scan (only valuable dirs)
//! Phase C: Hierarchical LLM inference

use fan_core::config::{Config, DataLayer};
use fan_core::discovery;
use fan_core::infer_hierarchical;
use fan_core::index::sqlite::SqliteStore;
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

    for root in &scan_roots {
        eprintln!("  Analyzing directory structure: {}", root);
        let result = if deep {
            discovery::run_recursive_phase_a(root, &llm_client, 9)
        } else {
            discovery::run_phase_a(root, &llm_client)
        };

        match result {
            Ok((targets, skips)) => {
                eprintln!("  → {} dirs to scan, {} skipped", targets.len(), skips.len());
                total_skipped += skips.len();
                for t in targets {
                    if t.starts_with('/') {
                        all_targets.push(t);
                    } else {
                        all_targets.push(format!("{}/{}", root.trim_end_matches('/'), t.trim_start_matches('/')));
                    }
                }
            }
            Err(e) => {
                eprintln!("  Phase A failed for {}: {}", root, e);
                eprintln!("  Falling back to full scan for this root.");
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

    // Open index
    let sqlite = match SqliteStore::open(&data_dir) {
        Ok(s) => {
            s.begin_batch().ok();
            s
        }
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    // Scan each target directory (using existing Scanner)
    let mut total_files = 0u64;
    for target in &all_targets {
        let scanner = fan_core::scanner::Scanner::new(
            vec![target.clone()],
            config.scan.exclude.clone(),
            "discovery".to_string(),
        );

        eprintln!("  Scanning: {}", target);
        for file_info in scanner.scan() {
            match sqlite.upsert(&file_info, None) {
                Ok(_) => total_files += 1,
                Err(e) => eprintln!("  Failed to index {}: {}", file_info.path.display(), e),
            }
        }
    }
    sqlite.commit_batch().ok();
    eprintln!("  Phase B complete: {} files indexed", total_files);
    println!();

    // ═══ Phase C: Hierarchical inference ═══
    println!("═══ Phase C: LLM Inference ═══");
    let project_store = ProjectStore::new(Arc::clone(&sqlite.conn));

    for root in &scan_roots {
        eprintln!("  Inferring: {}", root);
        match infer_hierarchical::run_hierarchical_inference(&sqlite, &project_store, &llm_client, root) {
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
