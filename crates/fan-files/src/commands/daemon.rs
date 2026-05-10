use fan_core::config::Config;
use fan_core::detector::BuiltinDetector;
use fan_core::index::IndexEngine;
use fan_core::plugin::registry::PluginRegistry;
use fan_core::scanner::Scanner;
use fan_core::watcher::FileWatcher;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub fn run(config: &Config) {
    info!("Starting fan-files daemon...");

    let index = IndexEngine::open(config).expect("Failed to open index engine");
    let mut plugins = PluginRegistry::new(config.plugins.dir.clone());
    let n = plugins.discover().unwrap_or(0);
    info!("Discovered {} plugins", n);

    let scanner = Scanner::new(config.scan.include.clone(), config.scan.exclude.clone());

    // Initial full scan
    run_full_scan(&index, &scanner, &plugins);

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
            run_full_scan(&index, &scanner, &plugins);
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
                                Ok(_) => info!("Re-indexed: {}", path.display()),
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

fn run_full_scan(index: &IndexEngine, scanner: &Scanner, plugins: &PluginRegistry) {
    info!("Starting full scan...");
    let start = Instant::now();
    let mut count = 0u64;
    for file_info in scanner.scan() {
        let path_str = file_info.path.to_string_lossy();
        let format_info = plugins
            .detect_format(&path_str, &file_info.magic_bytes)
            .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
        match index.index_file(&file_info, format_info.as_ref()) {
            Ok(_) => count += 1,
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
