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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub fn run(config: &Config, layer: &DataLayer, precise: bool) {
    run_inner(config, layer, precise, false);
}

pub fn run_re_infer(config: &Config, layer: &DataLayer, precise: bool) {
    run_inner(config, layer, precise, true);
}

fn run_inner(config: &Config, layer: &DataLayer, precise: bool, re_infer: bool) {
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

    // ═══ Phase A: Bottom-Up Discovery ═══
    println!("═══ Phase A: Bottom-Up Discovery ═══");
    let mut all_targets: Vec<String> = Vec::new();
    let mut all_uniform_dirs: Vec<discovery::UniformDir> = Vec::new();
    let mut all_dataset_candidates: Vec<discovery::DatasetCandidate> = Vec::new();
    let mut total_skipped = 0;

    for root in &scan_roots {
        eprintln!("  Analyzing directory structure: {}", root);
        let result = discovery::run_bottom_up_discovery(root, &llm_client);

        let result = match result {
            Err(e) => {
                eprintln!("  Phase A failed: {}. Falling back to full scan.", e);
                all_targets.push(root.to_string());
                continue;
            }
            ok => ok,
        };

        match result {
            Ok(discovery_result) => {
                eprintln!("  → {} dirs to scan, {} skipped, {} uniform",
                    discovery_result.targets.len(), discovery_result.skips.len(),
                    discovery_result.uniform_dirs.len());
                total_skipped += discovery_result.skips.len();
                for t in discovery_result.targets {
                    let abs = if t.starts_with('/') {
                        t.clone()
                    } else {
                        format!("{}/{}", root.trim_end_matches('/'), t.trim_start_matches('/'))
                    };
                    all_targets.push(abs);
                }
                all_uniform_dirs.extend(discovery_result.uniform_dirs);
                all_dataset_candidates.extend(discovery_result.dataset_candidates);
            }
            Err(e) => {
                eprintln!("  Phase A failed after retry: {}. Scanning root as-is.", e);
                all_targets.push(root.to_string());
            }
        }
    }

    // Build uniform-dir lookup: path_prefix → UniformDir
    let uniform_map: HashMap<String, discovery::UniformDir> = all_uniform_dirs
        .into_iter()
        .map(|u| (u.path.clone(), u))
        .collect();

    eprintln!(
        "  Phase A complete: {} targets, {} skipped, {} uniform, {} dataset candidates. {} roots.",
        all_targets.len(), total_skipped, uniform_map.len(), all_dataset_candidates.len(), scan_roots.len()
    );
    println!();

    // ═══ Phase B: Root-level scan with uniform-dir fast-path ═══
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
    let mut uniform_fastpath_count = 0u64;
    let mut filtered_skip_count = 0u64;
    index.sqlite.begin_batch().ok();

    // Collect Phase A BIO dirs for Phase C hints
    let phase_a_bio_dirs: std::collections::HashSet<String> = all_targets.iter().cloned().collect();

    // Collect uniform dir paths for fast O(1) lookup during scan
    let uniform_parents: std::collections::HashSet<String> = uniform_map.keys().cloned().collect();

    for root in &scan_roots {
        let scanner = fan_core::scanner::Scanner::new(
            vec![root.to_string()],
            config.scan.exclude.clone(),
            "discovery".to_string(),
        )
        .with_skip_magic(uniform_parents.clone())
        .with_precise_mode(precise);

        eprintln!("  Scanning root: {}", root);
        for file_info in scanner.scan() {
            let file_path = file_info.path.to_string_lossy();
            let parent = file_info.path.parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            // Uniform dir bulk file: Scanner already did open+read.
            // For now, count them but the real skip happens when we
            // bypass Scanner entirely (next iteration).
            let is_uniform_bulk = uniform_parents.contains(&parent);

            match index.index_file(&file_info, None) {
                Ok(_) => {
                    total_files += 1;
                    batch_count += 1;
                    if is_uniform_bulk { uniform_fastpath_count += 1; }
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
    eprintln!("  Phase B complete: {} files indexed ({} via uniform fast-path, {} filtered by targets)", total_files, uniform_fastpath_count, filtered_skip_count);
    println!();

    // Create snapshot for re-infer tracking
    let snapshot_id: Option<i64> = if re_infer {
        let prev = index.sqlite.latest_snapshot().ok().flatten();
        let prev_summary = prev.map(|(_, s)| s).unwrap_or_default();
        match index.sqlite.create_snapshot("manual", "", &format!("{} datasets", all_dataset_candidates.len())) {
            Ok(id) => {
                eprintln!("  Snapshot #{} created (previous: {})", id, prev_summary);
                Some(id)
            }
            Err(e) => { eprintln!("  Failed to create snapshot: {}", e); None }
        }
    } else { None };

    // ═══ Phase C: Hierarchical inference ═══
    println!("═══ Phase C: LLM Inference ═══");
    let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));

    // Phase C: Dataset + Asset inference
    // Fallback: if Phase A returned no candidates, use scan_roots directly
    let candidates: Vec<fan_core::discovery::DatasetCandidate> = if all_dataset_candidates.is_empty() {
        scan_roots.iter().map(|r| fan_core::discovery::DatasetCandidate {
            path: r.to_string(),
            dataset_type: "other".to_string(),
            species: None,
            confidence: "low".to_string(),
        }).collect()
    } else {
        all_dataset_candidates
    };

    eprintln!("  Inferring datasets from {} candidates...", candidates.len());
    match infer_hierarchical::run_dataset_asset_inference(
        &index.sqlite, &llm_client,
        scan_roots.first().unwrap_or(&""),
        &candidates
    ) {
        Ok(n) => eprintln!("  → {} datasets created", n),
        Err(e) => eprintln!("  Dataset inference failed: {}", e),
    }

    println!();
    println!("╔══════════════════════════════════════════╗");
    println!("║   Discovery Complete                     ║");
    println!("║   {} files indexed, {} targets scanned ║", total_files, all_targets.len());
    println!("╚══════════════════════════════════════════╝");
}
