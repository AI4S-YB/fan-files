use fan_core::config::Config;
use fan_core::detector::BuiltinDetector;
use fan_core::index::IndexEngine;
use fan_core::interpreter::InterpreterRegistry;
use fan_core::plugin::registry::PluginRegistry;
use fan_core::scanner::Scanner;
use fan_core::watcher::FileWatcher;
use fan_plugin_sdk::{FileContext, FormatInfo};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub fn run(config: &Config) {
    info!("Starting fan-files daemon...");

    let index = IndexEngine::open(config, false).expect("Failed to open index engine");
    let mut plugins = PluginRegistry::new(config.plugins.dir.clone());
    let n = plugins.discover().unwrap_or(0);
    info!("Discovered {} plugins", n);

    let interpreter_registry = InterpreterRegistry::new();

    let scanner = Scanner::new(config.scan.include.clone(), config.scan.exclude.clone());

    // Initial full scan
    run_full_scan(&index, &scanner, &plugins, &interpreter_registry);

    let sync_time = parse_sync_time(&config.schedule.full_sync);

    if config.watch.include.is_empty() {
        warn!("No watch directories configured, daemon exiting after scan");
        return;
    }

    let watcher = match FileWatcher::new(&config.watch.include) {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to start file watcher: {}", e);
            return;
        }
    };
    info!("File watcher started");
    info!(
        "Scheduled full sync at {:02}:{:02} UTC daily",
        sync_time.0, sync_time.1
    );

    let retention_days = config.retention.deleted_keep_days;
    let mut last_sync_day: Option<u64> = None;
    let mut last_purge_day: Option<u64> = None;

    loop {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let current_day = now_secs / 86400;
        let current_hour_min = hour_minute_from_secs(now_secs);

        // Scheduled full sync: check if we're in the sync hour window and haven't synced today
        if last_sync_day != Some(current_day)
            && current_hour_min.0 == sync_time.0
            && current_hour_min.1 >= sync_time.1
            && current_hour_min.1 < sync_time.1 + 10
        {
            info!("Running scheduled full sync...");
            run_full_scan(&index, &scanner, &plugins, &interpreter_registry);
            last_sync_day = Some(current_day);
        }

        // Daily purge of old deleted entries
        if last_purge_day != Some(current_day) {
            match index.sqlite.purge_old_deleted(retention_days) {
                Ok(n) if n > 0 => info!("Purged {} old deleted entries", n),
                Ok(_) => {}
                Err(e) => error!("Failed to purge old entries: {}", e),
            }
            last_purge_day = Some(current_day);
        }

        match watcher.events().recv_timeout(Duration::from_secs(10)) {
            Ok(paths) => {
                for path in &paths {
                    if path.exists() {
                        if let Some(file_info) = scanner.scan_single(path) {
                            let path_str = file_info.path.to_string_lossy();
                            let format_info = plugins
                                .detect_format(&path_str, &file_info.magic_bytes)
                                .or_else(|| {
                                    BuiltinDetector::detect(&path_str, &file_info.magic_bytes)
                                });
                            match index.index_file(&file_info, format_info.as_ref()) {
                                Ok(file_id) => {
                                    info!("Re-indexed: {}", path.display());
                                    // Run context interpretation after indexing
                                    run_interpretation(
                                        &index,
                                        file_id,
                                        file_info.path.as_ref(),
                                        format_info.as_ref(),
                                        &interpreter_registry,
                                    );
                                }
                                Err(e) => {
                                    error!("Failed to re-index {}: {}", path.display(), e)
                                }
                            }
                        }
                    } else {
                        // File was removed
                        if let Err(e) = index.sqlite.mark_deleted(path) {
                            error!("Failed to mark deleted {}: {}", path.display(), e);
                        } else {
                            info!("Marked deleted: {}", path.display());
                        }
                    }
                }
                // Commit after processing batch
                index.tantivy.commit().ok();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Normal timeout — loop continues to check schedule
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                error!("Watcher channel disconnected");
                break;
            }
        }
    }

    info!("Daemon shutting down");
}

fn run_full_scan(
    index: &IndexEngine,
    scanner: &Scanner,
    plugins: &PluginRegistry,
    interpreter_registry: &InterpreterRegistry,
) {
    info!("Starting full scan...");
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
                // Run context interpretation after indexing
                run_interpretation(
                    index,
                    file_id,
                    file_info.path.as_ref(),
                    format_info.as_ref(),
                    interpreter_registry,
                );
                // Generate and store embedding
                run_embedding(index, file_id, file_info.path.as_ref());
            }
            Err(e) => error!("Failed to index {}: {}", file_info.path.display(), e),
        }
    }
    index.tantivy.commit().ok();
    info!(
        "Full scan complete: {} files indexed in {:.1}s",
        count,
        start.elapsed().as_secs_f64()
    );
}

/// Build a FileContext from a file path, its format info, and its surrounding directory
fn build_file_context(
    file_path: &Path,
    format_info: Option<&FormatInfo>,
) -> FileContext {
    use fan_core::interpreter;

    FileContext {
        file_path: file_path.to_string_lossy().to_string(),
        siblings: interpreter::list_siblings(file_path),
        directory_tree: interpreter::directory_tree(file_path, 3),
        metadata_files: interpreter::find_metadata_files(file_path),
        file_header_b64: String::new(), // Skip for MVP
        format_tags: format_info
            .map(|f| vec![f.file_type.clone()])
            .unwrap_or_default(),
    }
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
