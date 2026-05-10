use fan_core::config::Config;
use fan_core::detector::BuiltinDetector;
use fan_core::index::IndexEngine;
use fan_core::plugin::registry::PluginRegistry;
use fan_core::scanner::Scanner;
use fan_core::watcher::FileWatcher;
use std::sync::mpsc;
use tracing::{error, info, warn};

pub fn run(config: &Config) {
    info!("Starting fan-files daemon...");

    let index = IndexEngine::open(config).expect("Failed to open index engine");
    let mut plugins = PluginRegistry::new(config.plugins.dir.clone());
    let n = plugins.discover().unwrap_or(0);
    info!("Discovered {} plugins", n);

    // Initial full scan
    info!("Starting initial scan of {:?}...", config.scan.include);
    let scanner = Scanner::new(config.scan.include.clone(), config.scan.exclude.clone());
    let mut scan_count = 0;

    for file_info in scanner.scan() {
        let path_str = file_info.path.to_string_lossy();
        let format_info = plugins.detect_format(&path_str, &file_info.magic_bytes)
            .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
        match index.index_file(&file_info, format_info.as_ref()) {
            Ok(_) => scan_count += 1,
            Err(e) => error!("Failed to index {}: {}", file_info.path.display(), e),
        }
    }
    index.tantivy.commit().ok();
    info!("Initial scan complete: {} files indexed", scan_count);

    // Start file watcher
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
    info!("File watcher started, monitoring changes...");

    loop {
        match watcher.events().recv() {
            Ok(paths) => {
                for path in &paths {
                    if path.exists() {
                        if let Some(file_info) = scanner.scan_single(path) {
                            let path_str = file_info.path.to_string_lossy();
                            let format_info = plugins.detect_format(&path_str, &file_info.magic_bytes)
                                .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                            match index.index_file(&file_info, format_info.as_ref()) {
                                Ok(_) => info!("Re-indexed: {}", path.display()),
                                Err(e) => error!("Failed to re-index {}: {}", path.display(), e),
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
            Err(mpsc::RecvError) => {
                error!("Watcher channel disconnected");
                break;
            }
        }
    }

    info!("Daemon shutting down");
}
