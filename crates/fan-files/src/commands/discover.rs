//! `fan-files discover` — Progressive Discovery pipeline.
//!
//! Phase A: Directory analysis (bottom-up with --deep, top-down without)
//! Phase B: Targeted file scan (merged targets to avoid per-leaf walkdir)
//! Phase C: Hierarchical LLM inference

use fan_core::config::{Config, DataLayer};
use std::thread;
use fan_core::discovery;
use fan_core::infer_hierarchical;
use fan_core::index::IndexEngine;
use fan_core::llm::LlmClient;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub fn run(config: &Config, layer: &DataLayer, precise: bool) {
    run_inner(config, layer, precise, false, false);
}

pub fn run_re_infer(config: &Config, layer: &DataLayer, precise: bool) {
    run_inner(config, layer, precise, true, false);
}

/// GUI 桌面壳专用：只扫本机 [scan].include 目录，忽略远程 [servers.*]。
pub fn run_local(config: &Config, layer: &DataLayer, precise: bool) {
    run_inner(config, layer, precise, false, true);
}

fn run_inner(config: &Config, layer: &DataLayer, precise: bool, re_infer: bool, local_only: bool) {
    let llm_client = LlmClient::new(config.llm.clone());
    if !llm_client.is_configured() {
        eprintln!("LLM not configured. Set [llm] in config.toml.");
        return;
    }

    // GUI 桌面壳只扫本机目录，忽略远程 [servers.*] 配置，
    // 否则 enabled_servers() 会返回远程 scan_roots，导致在本地扫不存在的路径。
    let scan_roots: Vec<String> = if local_only || config.enabled_servers().is_empty() {
        config.scan.include.clone()
    } else {
        let servers = config.enabled_servers();
        servers.iter()
            .flat_map(|(_, cfg)| cfg.scan_roots.iter().cloned())
            .collect()
    };
    if scan_roots.is_empty() {
        eprintln!("No scan roots configured. Use 'fan-files servers add' or set [scan] include.");
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

    // Parallel root scanning — SqliteStore is Sync (internal Mutex).
    // Each root scanned in its own thread, SQLite writes are serialized by Mutex.
    let sqlite_ref = &index.sqlite;
    let scan_exclude = config.scan.exclude.clone();
    let up = uniform_parents.clone();
    let n_threads = config.threads.unwrap_or_else(|| scan_roots.len().min(10));
    for chunk in scan_roots.chunks(n_threads.max(1)) {
        thread::scope(|s| {
            let mut handles = Vec::new();
            for root in chunk {
                let root = root.to_string();
                let exclude = scan_exclude.clone();
                let up2 = up.clone();
                handles.push(s.spawn(move || {
                    let scanner = fan_core::scanner::Scanner::new(
                        vec![root.clone()],
                        exclude,
                        "discovery".to_string(),
                    )
                    .with_skip_magic(up2)
                    .with_precise_mode(precise);

                    let mut local_files = 0u64;
                    let mut local_batch = 0usize;
                    sqlite_ref.begin_batch().ok();

                    for info in scanner.scan() {
                        match sqlite_ref.upsert(&info, None) {
                            Ok(_) => { local_files += 1; local_batch += 1; }
                            Err(e) => eprintln!("  Failed to index {}: {}",
                                info.path.display(), e),
                        }
                        if local_batch >= 1000 {
                            sqlite_ref.commit_batch().ok();
                            local_batch = 0;
                            sqlite_ref.begin_batch().ok();
                        }
                    }
                    if local_batch > 0 { sqlite_ref.commit_batch().ok(); }
                    eprintln!("  Root {}: {} files indexed", root, local_files);
                    local_files
                }));
            }
            for h in handles {
                if let Ok(f) = h.join() { total_files += f; }
            }
        });
    }
    eprintln!("  Phase B complete: {} files indexed", total_files);
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

    // Phase C: Dataset + Asset inference
    // Fallback: if Phase A returned no candidates, use scan_roots directly
    let candidates: Vec<fan_core::discovery::DatasetCandidate> = if all_dataset_candidates.is_empty() {
        scan_roots.iter().map(|r| fan_core::discovery::DatasetCandidate {
            path: r.to_string(),
            dataset_type: "other".to_string(),
            species: None,
            confidence: "low".to_string(),
            candidate_role: None,
        }).collect()
    } else {
        all_dataset_candidates
    };

    eprintln!("  Inferring datasets from {} candidates...", candidates.len());
    match infer_hierarchical::run_dataset_asset_inference(
        &index.sqlite, &llm_client, &index.tantivy, config.threads,
        scan_roots.first().map(|s| s.as_str()).unwrap_or(""),
        &candidates
    ) {
        Ok(n) => eprintln!("  → {} datasets created", n),
        Err(e) => eprintln!("  Dataset inference failed: {}", e),
    }

    // Rebuild Tantivy search index with Phase C metadata
    eprintln!("  Building search index...");
    if let (Ok(all_files), Ok(all_datasets)) =
        (index.sqlite.all_paths(), index.sqlite.all_datasets())
    {
        // Sort datasets by path length descending (most specific first → break early)
        let mut ds_info: Vec<(&str, &str, &str)> = all_datasets.iter().map(|d| {
            (d.path.as_str(),
             d.dataset_type.as_deref().unwrap_or("?"),
             d.species.as_deref().unwrap_or("?"))
        }).collect();
        ds_info.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (id, path, _size) in &all_files {
            let metadata = path.clone();
            for (ds_path, ds_type, ds_species) in &ds_info {
                if path.starts_with(ds_path) {
                    let md = format!("{} | dataset:{} | type:{} | species:{}",
                        path, ds_path, ds_type, ds_species);
                    index.tantivy.index_file(*id, &std::path::Path::new(path),
                        &md, &[ds_path]).ok();
                    break;
                }
            }
        }
    }
    index.tantivy.commit().ok();

    println!();
    println!("╔══════════════════════════════════════════╗");
    println!("║   Discovery Complete                     ║");
    println!("║   {} files indexed, {} targets scanned ║", total_files, all_targets.len());
    println!("╚══════════════════════════════════════════╝");
}
