use fan_core::config::Config;
use fan_core::index::IndexEngine;

pub fn run(config: &Config) {
    let index = match IndexEngine::open(config, true) {
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

            // Show metadata coverage
            let with_meta = index.sqlite.count_with_bio_metadata().unwrap_or(0);
            let pct = if status.indexed_files > 0 {
                (with_meta as f64 / status.indexed_files as f64) * 100.0
            } else {
                0.0
            };
            println!("Metadata coverage: {:.0}% ({}/{})", pct, with_meta, status.indexed_files);
            if pct < 50.0 && status.indexed_files > 10 {
                println!("  ⚠ Metadata coverage is low. Run 'fan-files infer' for better search results.");
            }

            let fmt_ts = |ts: i64| -> String {
                std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(ts as u64))
                    .map(|t| format!("{:?}", t))
                    .unwrap_or_else(|| ts.to_string())
            };

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
