use fan_core::config::Config;

pub fn run(config: &Config) {
    let index = match fan_core::index::open_index(config, fan_core::index::IndexMode::ReadOnly) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    match index.sqlite.status() {
        Ok(status) => {
            println!("fan-files Index Status");
            println!("====================");
            println!("Indexed files:   {}", status.indexed_files);
            println!("Total tracked:   {}", status.total_files);
            println!("Deleted (soft):  {}", status.deleted_files);

            let with_meta = index.sqlite.count_with_bio_metadata().unwrap_or(0);
            let pct = if status.indexed_files > 0 {
                (with_meta as f64 / status.indexed_files as f64) * 100.0
            } else {
                0.0
            };
            println!("Metadata coverage: {:.0}% ({}/{})", pct, with_meta, status.indexed_files);
            if pct < 50.0 && status.indexed_files > 10 {
                println!("  ⚠ Metadata coverage is low. Run 'fan-files infer' for better results.");
            }

            let fmt_ts = |ts: i64| -> String {
                std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(ts as u64))
                    .map(|t| format!("{:?}", t))
                    .unwrap_or_else(|| ts.to_string())
            };

            // Per-server breakdown
            match index.sqlite.status_by_server() {
                Ok(servers) => {
                    if !servers.is_empty() {
                        println!();
                        println!("Servers:");
                        let max_name = servers.iter().map(|s| s.server.len()).max().unwrap_or(6);
                        for s in &servers {
                            let last = s.last_scan.map(fmt_ts).unwrap_or_else(|| "never".to_string());
                            println!(
                                "  {:<width$} {:>8} files  (last scan: {})",
                                s.server,
                                s.file_count,
                                last,
                                width = max_name + 2,
                            );
                        }
                    }
                }
                Err(_) => {}
            }

            if let Some(ts) = status.last_full_scan {
                println!("Last scan:       {}", fmt_ts(ts));
            }
            if let Some(ts) = status.last_change {
                println!("Last change:     {}", fmt_ts(ts));
            }
        }
        Err(e) => eprintln!("Error querying status: {}", e),
    }
}
