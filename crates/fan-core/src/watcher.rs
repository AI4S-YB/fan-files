use notify::{Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<Vec<PathBuf>>,
}

#[derive(Debug, Clone)]
pub enum ChangeEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

impl FileWatcher {
    pub fn new(dirs: &[String]) -> notify::Result<Self> {
        let (raw_tx, raw_rx) = mpsc::channel();
        let (batch_tx, batch_rx) = mpsc::channel::<Vec<PathBuf>>();

        // Batching thread: dedup and batch events
        std::thread::spawn(move || {
            let mut batch: Vec<PathBuf> = Vec::new();
            let mut last_flush = Instant::now();
            loop {
                match raw_rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(path) => {
                        if !batch.contains(&path) {
                            batch.push(path);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if !batch.is_empty()
                    && (batch.len() >= 100 || last_flush.elapsed() > Duration::from_secs(5))
                {
                    let drained: Vec<PathBuf> = batch.drain(..).collect();
                    debug!("Flushing {} changed paths", drained.len());
                    batch_tx.send(drained).ok();
                    last_flush = Instant::now();
                }
            }
        });

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                match res {
                    Ok(event) => {
                        match event.kind {
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                                for path in event.paths {
                                    raw_tx.send(path).ok();
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => warn!("Watch error: {}", e),
                }
            },
            NotifyConfig::default(),
        )?;

        for dir in dirs {
            let path = Path::new(dir);
            if path.exists() {
                watcher.watch(path, RecursiveMode::Recursive)?;
                info!("Watching: {}", dir);
            } else {
                warn!("Watch directory does not exist: {}", dir);
            }
        }

        Ok(Self { _watcher: watcher, rx: batch_rx })
    }

    /// Returns the receiver for batches of changed paths
    pub fn events(&self) -> &Receiver<Vec<PathBuf>> {
        &self.rx
    }
}
