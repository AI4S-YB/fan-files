use std::collections::HashMap;
use std::sync::Mutex;

use fan_core::config::{Config, DataLayer};
use fan_core::detector::BuiltinDetector;
use fan_core::infer;
use fan_core::index::IndexEngine;
use fan_core::interpreter::InterpreterRegistry;
use fan_core::llm::LlmClient;
use fan_core::plugin::registry::PluginRegistry;
use fan_core::project::ProjectStore;
use fan_core::scanner::RemoteScanner;
use fan_core::scanner::Scanner;
use fan_core::watcher::FileWatcher;
use fan_plugin_sdk::{FileContext, FormatInfo};
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub fn run(config: &Config, layer: &DataLayer) {
    run_inner(config, layer, false);
}

pub fn run_scan_only(config: &Config, layer: &DataLayer) {
    run_inner(config, layer, true);
}

fn run_inner(config: &Config, layer: &DataLayer, scan_only: bool) {
    info!("Starting fan-files daemon...");

    let index = match fan_core::index::open_index_for_layer(config, layer, fan_core::index::IndexMode::ReadWrite) {
        Ok(i) => i,
        Err(e) => {
            error!("Failed to open index engine: {}", e);
            return;
        }
    };
    let mut plugins = PluginRegistry::new(config.plugins.dir.clone());
    let n = plugins.discover().unwrap_or(0);
    info!("Discovered {} plugins", n);

    let interpreter_registry = InterpreterRegistry::new();

    let servers = config.enabled_servers();
    let local_server_names: Vec<String> = servers
        .iter()
        .filter(|(_, cfg)| cfg.host.is_empty())
        .map(|(n, _)| n.clone())
        .collect();

    // Initial full scan: local + remote
    for (name, cfg) in &servers {
        if cfg.host.is_empty() {
            let scanner = Scanner::new(
                cfg.scan_roots.clone(),
                config.scan.exclude.clone(),
                name.clone(),
            );
            info!("Starting local scan: {} ({})", name, cfg.scan_roots.join(", "));
            run_full_scan(&index, &scanner, &plugins, &interpreter_registry, scan_only);
        } else {
            let remote = RemoteScanner::new(
                name.clone(),
                cfg.host.clone(),
                cfg.scan_roots.clone(),
            );
            info!("Starting remote scan: {} ({})", name, cfg.scan_roots.join(", "));
            match remote.scan(false) {
                Ok(entries) => {
                    let start = Instant::now();
                    let mut count = 0u64;
                    for file_info in &entries {
                        let path_str = file_info.path.to_string_lossy();
                        let format_info = plugins
                            .detect_format(&path_str, &file_info.magic_bytes)
                            .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                        match index.index_file(file_info, format_info.as_ref()) {
                            Ok(_) => count += 1,
                            Err(e) => error!("Failed to index {}: {}", file_info.path.display(), e),
                        }
                    }
                    index.tantivy.commit().ok();
                    info!(
                        "Remote scan complete: {} ({}) — {} files in {:.1}s",
                        name, cfg.scan_roots.join(", "), count, start.elapsed().as_secs_f64()
                    );
                }
                Err(e) => error!("Remote scan failed for {}: {}", name, e),
            }
        }
    }

    // After initial scan, run LLM inference if configured
    {
        let llm_client = LlmClient::new(config.llm.clone());
        if llm_client.is_configured() {
            let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));
            let scan_root = servers.first().and_then(|(_, c)| c.scan_roots.first().map(|s| s.as_str())).unwrap_or("/");
            info!("Running LLM inference on indexed files...");
            match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
                Ok((p, r)) => info!("LLM inference complete: {} projects, {} relations", p, r),
                Err(e) => warn!("LLM inference failed: {}", e),
            }
        }
    }

    // File watcher: only for local servers
    if local_server_names.is_empty() {
        warn!("No local servers — file watcher disabled, daemon exiting after scan");
        return;
    }

    let watch_dirs: Vec<String> = servers
        .iter()
        .filter(|(_, cfg)| cfg.host.is_empty())
        .flat_map(|(_, cfg)| cfg.scan_roots.clone())
        .collect();

    let watcher = match FileWatcher::new(&watch_dirs) {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to start file watcher: {}", e);
            return;
        }
    };
    info!("File watcher started for local servers");

    let sync_time = parse_sync_time(&config.schedule.full_sync);
    let retention_days = config.retention.deleted_keep_days;
    let mut last_sync_day: Option<u64> = None;
    let mut last_purge_day: Option<u64> = None;
    let mut new_files_since_infer: u64 = 0;

    loop {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let current_day = now_secs / 86400;
        let current_hour_min = hour_minute_from_secs(now_secs);

        if last_sync_day != Some(current_day)
            && current_hour_min.0 == sync_time.0
            && current_hour_min.1 >= sync_time.1
            && current_hour_min.1 < sync_time.1 + 10
        {
            info!("Running scheduled full sync...");
            // Rescan local
            for name in &local_server_names {
                if let Some((_, cfg)) = servers.iter().find(|(n, _)| n == name) {
                    let scanner = Scanner::new(
                        cfg.scan_roots.clone(),
                        config.scan.exclude.clone(),
                        name.clone(),
                    );
                    run_full_scan(&index, &scanner, &plugins, &interpreter_registry, scan_only);
                }
            }
            // Rescan remote
            for (name, cfg) in &servers {
                if !cfg.host.is_empty() {
                    let remote = RemoteScanner::new(
                        name.clone(),
                        cfg.host.clone(),
                        cfg.scan_roots.clone(),
                    );
                    match remote.scan(false) {
                        Ok(entries) => {
                            let mut count = 0u64;
                            for file_info in &entries {
                                let path_str = file_info.path.to_string_lossy();
                                let format_info = plugins
                                    .detect_format(&path_str, &file_info.magic_bytes)
                                    .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                                if let Ok(_) = index.index_file(file_info, format_info.as_ref()) {
                                    count += 1;
                                }
                            }
                            index.tantivy.commit().ok();
                            info!("Scheduled remote scan: {} — {} files", name, count);
                        }
                        Err(e) => warn!("Scheduled remote scan failed for {}: {}", name, e),
                    }
                }
            }

            // LLM after scheduled scan
            {
                let llm_client = LlmClient::new(config.llm.clone());
                if llm_client.is_configured() {
                    let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));
                    let scan_root = servers.first().and_then(|(_, c)| c.scan_roots.first().map(|s| s.as_str())).unwrap_or("/");
                    match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
                        Ok((p, r)) => info!("LLM inference: {} projects, {} relations", p, r),
                        Err(e) => warn!("LLM inference failed: {}", e),
                    }
                }
            }
            new_files_since_infer = 0;
            last_sync_day = Some(current_day);
        }

        if last_purge_day != Some(current_day) {
            match index.sqlite.purge_old_deleted(retention_days) {
                Ok(n) if n > 0 => info!("Purged {} old deleted entries", n),
                Ok(_) => {}
                Err(e) => error!("Failed to purge: {}", e),
            }
            last_purge_day = Some(current_day);
        }

        match watcher.events().recv_timeout(Duration::from_secs(10)) {
            Ok(paths) => {
                for path in &paths {
                    if path.exists() {
                        if let Some(ref local_name) = local_server_names.first() {
                            let file_info = read_local_file_info(path, local_name);
                            let path_str = file_info.path.to_string_lossy();
                            let format_info = plugins
                                .detect_format(&path_str, &file_info.magic_bytes)
                                .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                            match index.index_file(&file_info, format_info.as_ref()) {
                                Ok(_) => {
                                    info!("Re-indexed: {}", path.display());
                                    new_files_since_infer += 1;
                                    if new_files_since_infer >= 10 {
                                        auto_infer(&index, config, &servers);
                                        new_files_since_infer = 0;
                                    }
                                }
                                Err(e) => error!("Failed to re-index {}: {}", path.display(), e),
                            }
                        }
                    } else {
                        if let Err(e) = index.sqlite.mark_deleted(path) {
                            error!("Failed to mark deleted {}: {}", path.display(), e);
                        } else {
                            info!("Marked deleted: {}", path.display());
                        }
                    }
                }
                index.tantivy.commit().ok();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                error!("Watcher channel disconnected");
                break;
            }
        }
    }

    info!("Daemon shutting down");
}

fn read_local_file_info(path: &std::path::Path, server_name: &str) -> fan_core::types::RawFileInfo {
    let meta = std::fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let magic = read_magic_local(path);
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    fan_core::types::RawFileInfo {
        path: path.to_path_buf(),
        source_server: server_name.to_string(),
        size,
        mtime_secs: mtime,
        hash_sha256: None,
        magic_bytes: magic,
        mime_type: mime,
    }
}

fn read_magic_local(path: &std::path::Path) -> Vec<u8> {
    std::fs::File::open(path)
        .ok()
        .and_then(|mut f| {
            use std::io::Read;
            let mut buf = vec![0u8; 512];
            let n = f.read(&mut buf).ok()?;
            buf.truncate(n);
            Some(buf)
        })
        .unwrap_or_default()
}

fn auto_infer(index: &IndexEngine, config: &Config, servers: &[(String, fan_core::config::ServerConfig)]) {
    let llm_client = LlmClient::new(config.llm.clone());
    if llm_client.is_configured() {
        let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));
        let scan_root = servers.first().and_then(|(_, c)| c.scan_roots.first().map(|s| s.as_str())).unwrap_or("/");
        match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
            Ok((p, r)) => info!("Auto-infer: {} projects, {} relations", p, r),
            Err(e) => warn!("Auto-infer failed: {}", e),
        }
    }
}

fn run_full_scan(
    index: &IndexEngine,
    scanner: &Scanner,
    plugins: &PluginRegistry,
    interpreter_registry: &InterpreterRegistry,
    scan_only: bool,
) {
    let mode = if scan_only { "scan-only" } else { "full" };
    info!("Starting {} scan...", mode);
    let start = Instant::now();
    let mut count = 0u64;
    for file_info in scanner.scan() {
        let path_str = file_info.path.to_string_lossy();
        let format_info = plugins
            .detect_format(&path_str, &file_info.magic_bytes)
            .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
        match index.index_file(&file_info, format_info.as_ref()) {
            Ok(file_id) => {
                count += 1;
                if !scan_only {
                    run_interpretation(
                        index, file_id, file_info.path.as_ref(),
                        format_info.as_ref(), interpreter_registry,
                    );
                    run_embedding(index, file_id, file_info.path.as_ref());
                }
            }
            Err(e) => error!("Failed to index {}: {}", file_info.path.display(), e),
        }
    }
    index.tantivy.commit().ok();
    info!(
        "{} scan complete: {} files indexed in {:.1}s",
        mode, count, start.elapsed().as_secs_f64()
    );
}

/// Build a FileContext from a file path, its format info, and its surrounding directory.
/// NOTE: siblings is always empty — list_siblings() is O(N) per directory and was
/// never consumed by any interpreter. Kept as empty for backward compat.
fn build_file_context(
    file_path: &Path,
    format_info: Option<&FormatInfo>,
) -> FileContext {
    use fan_core::interpreter;

    FileContext {
        file_path: file_path.to_string_lossy().to_string(),
        siblings: Vec::new(), // Dead code — no interpreter reads this; removing O(N) list_siblings call
        directory_tree: interpreter::directory_tree(file_path, 3),
        metadata_files: find_metadata_files_cached(file_path),
        file_header_b64: String::new(),
        format_tags: format_info
            .map(|f| vec![f.file_type.clone()])
            .unwrap_or_default(),
    }
}

/// Cached wrapper around interpreter::find_metadata_files.
/// Same directory's metadata files are identical — cache per parent dir.
fn find_metadata_files_cached(file_path: &Path) -> Vec<String> {
    use std::sync::LazyLock;
    static CACHE: LazyLock<Mutex<HashMap<std::path::PathBuf, Vec<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    let parent = file_path.parent().unwrap_or(Path::new("/"));
    let mut cache = CACHE.lock().unwrap();
    cache
        .entry(parent.to_path_buf())
        .or_insert_with(|| fan_core::interpreter::find_metadata_files(file_path))
        .clone()
}

/// Run context interpretation on an indexed file and store bio metadata
fn run_interpretation(
    index: &IndexEngine,
    file_id: i64,
    file_path: &Path,
    format_info: Option<&FormatInfo>,
    interpreter_registry: &InterpreterRegistry,
) {
    let ctx = build_file_context(file_path, format_info);

    if let Some(bio_meta) = interpreter_registry.best_interpretation(&ctx, 0.3) {
        if let Err(e) = index.sqlite.update_bio_metadata(file_id, &bio_meta) {
            error!(
                "Failed to update bio metadata for {}: {}",
                file_path.display(),
                e
            );
        } else if bio_meta.assay_type.is_some() || bio_meta.species.is_some() {
            info!(
                "Bio metadata inferred for {}: assay={:?}, species={:?}, tags={:?}",
                file_path.display(),
                bio_meta.assay_type,
                bio_meta.species,
                bio_meta.tags,
            );
        }
    }
}

/// Generate embedding vector for a file and store it
fn run_embedding(index: &IndexEngine, file_id: i64, file_path: &Path) {
    if !index.embedding.is_available() {
        return;
    }

    let text = build_embedding_text(index, file_id, file_path);
    match index.embedding.embed(&text) {
        Ok(vec) => {
            if let Err(e) = index.sqlite.store_embedding(file_id, &vec) {
                error!("Failed to store embedding for {}: {}", file_path.display(), e);
            }
        }
        Err(_e) => {
            // Binary files or files with no extractable text naturally fail embedding — not an error
        }
    }
}

/// Build a text representation of a file for embedding generation.
/// Combines filename, format, bio metadata for richer semantic representation.
fn build_embedding_text(index: &IndexEngine, file_id: i64, file_path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Filename (without full path)
    if let Some(name) = file_path.file_name().and_then(|n| n.to_str()) {
        parts.push(name.to_string());
    }

    // File type from format info
    if let Ok(Some(entry)) = index.sqlite.get_by_id(file_id) {
        if let Some(ref fmt) = entry.format_info {
            parts.push(fmt.file_type.clone());
        }

        // Bio metadata fields
        if let Some(ref meta) = entry.bio_metadata {
            if let Some(ref assay) = meta.assay_type {
                parts.push(assay.clone());
            }
            if let Some(ref species) = meta.species {
                parts.push(species.clone());
            }
            if let Some(ref tissue) = meta.tissue {
                parts.push(tissue.clone());
            }
            if let Some(ref project) = meta.project {
                parts.push(project.clone());
            }
            for tag in &meta.tags {
                parts.push(tag.clone());
            }
        }

        // Parent directory names (contain contextual info)
        for ancestor in file_path.ancestors().skip(1).take(2) {
            if let Some(name) = ancestor.file_name().and_then(|n| n.to_str()) {
                if !name.is_empty() && name != "/" {
                    parts.push(name.to_string());
                }
            }
        }
    }

    parts.join(" ")
}

fn parse_sync_time(s: &str) -> (u32, u32) {
    let parts: Vec<&str> = s.split(':').collect();
    let hour = parts.first().and_then(|p| p.parse().ok()).unwrap_or(3);
    let minute = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    (hour, minute)
}

fn hour_minute_from_secs(total_secs: u64) -> (u32, u32) {
    let total_mins = total_secs / 60;
    let hour = ((total_mins / 60) % 24) as u32;
    let minute = (total_mins % 60) as u32;
    (hour, minute)
}
