//! fan-agent — lightweight file scanning + watching agent for remote Linux servers.
//!
//! Commands:
//!   fan-agent scan --root <path>              # one-shot full scan → JSONL
//!   fan-agent scan --root <path> --since <ts> # incremental (mtime > ts)
//!   fan-agent watch --root <path>             # real-time inotify watch → JSONL

use serde::Serialize;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

#[derive(Serialize)]
struct FileEntry {
    path: String,
    size: u64,
    mtime_secs: i64,
}

#[derive(Serialize)]
struct WatchEvent {
    event: String,  // "add" | "mod" | "del"
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mtime_secs: Option<i64>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage:");
        eprintln!("  fan-agent scan --root <path> [--since <unix_ts>]");
        eprintln!("  fan-agent watch --root <path>");
        std::process::exit(1);
    }

    let cmd = &args[1];
    match cmd.as_str() {
        "scan" => cmd_scan(&args),
        "watch" => cmd_watch(&args),
        _ => {
            eprintln!("Unknown command: {}", cmd);
            std::process::exit(1);
        }
    }
}

fn cmd_scan(args: &[String]) {
    let mut root = String::from("/");
    let mut since: Option<i64> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                if i < args.len() { root = args[i].clone(); }
            }
            "--since" => {
                i += 1;
                if i < args.len() { since = args[i].parse().ok(); }
            }
            _ => {}
        }
        i += 1;
    }

    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

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
        let mtime = meta.modified().ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64).unwrap_or(0);

        if let Some(s) = since { if mtime <= s { continue; } }

        let fe = FileEntry {
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            mtime_secs: mtime,
        };
        serde_json::to_writer(&mut writer, &fe).ok();
        writeln!(writer).ok();
    }
    writer.flush().ok();
}

fn cmd_watch(args: &[String]) {
    use notify::{Event, EventKind, RecursiveMode, Watcher};

    let mut root = String::from("/");
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                if i < args.len() { root = args[i].clone(); }
            }
            _ => {}
        }
        i += 1;
    }

    eprintln!("fan-agent watching: {} (press Ctrl+C to stop)", root);

    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res { tx.send(event).ok(); }
    }).expect("Failed to create watcher");

    watcher.watch(Path::new(&root), RecursiveMode::Recursive)
        .expect("Failed to watch directory");

    for event in rx {
        let event_kind = &event.kind;
        let is_create = matches!(event_kind, EventKind::Create(_));
        let is_modify = matches!(event_kind, EventKind::Modify(_));
        let is_remove = matches!(event_kind, EventKind::Remove(_));

        for path in &event.paths {
            let path_str = path.to_string_lossy().to_string();

            if is_remove {
                let ev = WatchEvent { event: "del".into(), path: path_str, size: None, mtime_secs: None };
                serde_json::to_writer(&mut writer, &ev).ok();
                writeln!(writer).ok();
            } else if is_create || is_modify {
                if !path.is_file() { continue; }
                let (size, mtime) = match path.metadata() {
                    Ok(m) => {
                        let mt = m.modified().ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64).unwrap_or(0);
                        (m.len(), mt)
                    }
                    Err(_) => continue,
                };
                let ev = WatchEvent {
                    event: if is_create { "add" } else { "mod" }.into(),
                    path: path_str,
                    size: Some(size),
                    mtime_secs: Some(mtime),
                };
                serde_json::to_writer(&mut writer, &ev).ok();
                writeln!(writer).ok();
            }
        }
        writer.flush().ok();
    }
}
