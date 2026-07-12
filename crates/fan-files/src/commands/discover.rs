//! `fan-files discover` — Progressive Discovery pipeline.
//!
//! Phase A: Directory analysis (bottom-up with --deep, top-down without)
//! Phase B: Targeted file scan (merged targets to avoid per-leaf walkdir)
//! Phase C: Hierarchical LLM inference

use fan_core::config::{Config, DataLayer};
use fan_core::discovery;
use fan_core::infer_hierarchical;
use fan_core::index::IndexEngine;
use fan_core::llm::LlmClient;
use fan_core::project::ProjectStore;
use std::collections::HashSet;
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

    // ═══ Phase A ═══
    let mode_label = if deep {
        "Bottom-Up (full depth + propagate up)"
    } else {
        "Shallow (depth 3)"
    };
    println!("═══ Phase A: Directory Analysis ({}) ═══", mode_label);
    let mut all_targets: Vec<String> = Vec::new();
    let mut total_skipped = 0;

    for root in &scan_roots {
        eprintln!("  Analyzing directory structure: {}", root);
        let result = if deep {
            discovery::run_bottom_up_discovery(root, &llm_client)
        } else {
            discovery::run_phase_a(root, &llm_client)
        };

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
        "  Phase A complete: {} raw targets, {} skipped",
        all_targets.len(), total_skipped
    );

    // ═══ Build skip set from Phase A results ═══
    // Phase A returns targets (dirs to scan). Everything NOT in targets
    // under a scan_root is a skip candidate. We track which roots had
    // Phase A succeed vs fallback.
    let skip_set: HashSet<String> = {
        // For roots where Phase A succeeded, everything not in targets is skipped
        // For now: we scan roots fully and skip nothing
        // (skip set is empty — Phase A guides which dirs become projects in Phase C)
        HashSet::new()
    };
    eprintln!(
        "  Phase A complete: {} raw targets, {} skipped. Scanning {} roots directly.",
        all_targets.len(), total_skipped, scan_roots.len()
    );
    println!();

    // ═══ Phase B: Root-level scan (daemon-style, 1 walkdir per root) ═══
    println!("═══ Phase B: Root-Level Scan ({} roots) ═══", scan_roots.len());
    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };

    let index = match IndexEngine::open_at(&data_dir, config, false) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    let mut total_files = 0u64;
    let mut batch_count = 0usize;
    index.sqlite.begin_batch().ok();

    for root in &scan_roots {
        let scanner = fan_core::scanner::Scanner::new(
            vec![root.to_string()],
            config.scan.exclude.clone(),
            "discovery".to_string(),
        );

        eprintln!("  Scanning root: {}", root);
        for file_info in scanner.scan() {
            // Phase A skip filter: check if file's parent tree is in skip set
            if !skip_set.is_empty() {
                let file_path = file_info.path.to_string_lossy();
                let skipped = skip_set.iter().any(|s| file_path.starts_with(s));
                if skipped { continue; }
            }

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
