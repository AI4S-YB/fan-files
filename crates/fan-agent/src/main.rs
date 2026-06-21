//! fan-agent — lightweight file scanning agent for remote Linux servers.
//!
//! This binary is designed to be deployed to remote servers (via scp/ssh)
//! and run locally there. It walks the filesystem and outputs file metadata
//! as JSONL to stdout, which the parent fan-files process reads and indexes.
//!
//! Usage:
//!   fan-agent scan --root /data/biodata              # full scan
//!   fan-agent scan --root /data/biodata --since 1000  # incremental (mtime > since)
//!   fan-agent scan --root /data/biodata --watch       # daemon mode (future)

use serde::Serialize;
use std::io::{self, Write};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

#[derive(Serialize)]
struct FileEntry {
    path: String,
    size: u64,
    mtime_secs: i64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 || args[1] != "scan" {
        eprintln!("Usage: fan-agent scan --root <path> [--since <unix_ts>]");
        std::process::exit(1);
    }

    let mut root = String::from("/");
    let mut since: Option<i64> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                if i < args.len() {
                    root = args[i].clone();
                }
            }
            "--since" => {
                i += 1;
                if i < args.len() {
                    since = args[i].parse().ok();
                }
            }
            _ => {}
        }
        i += 1;
    }

    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());

    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let meta = match path.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Skip if mtime is not newer than --since
        if let Some(s) = since {
            if mtime <= s {
                continue;
            }
        }

        let entry = FileEntry {
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            mtime_secs: mtime,
        };

        serde_json::to_writer(&mut writer, &entry).ok();
        writeln!(writer).ok();
    }

    writer.flush().ok();
}
