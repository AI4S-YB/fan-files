use fan_core::config::{Config, DataLayer};

pub fn run(config: &Config, layer: &DataLayer) {
    let index = match fan_core::index::open_index_for_layer(config, layer, fan_core::index::IndexMode::ReadOnly) {
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
                timestamp_to_str(ts)
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

fn timestamp_to_str(ts: i64) -> String {
    if ts <= 0 { return "never".to_string(); }
    // Convert unix timestamp to YYYY-MM-DD HH:MM:SS
    let secs_per_day: i64 = 86400;
    let days = ts / secs_per_day;
    let time_of_day = ts % secs_per_day;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Compute year/month/day from days since epoch (1970-01-01)
    let (y, m, d) = days_to_ymd(days);

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hours, minutes, seconds)
}

/// Convert days since 1970-01-01 to (year, month, day).
fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719468; // shift to 0000-03-01 epoch (for easier leap year math)
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { (mp + 3) as u32 } else { (mp - 9) as u32 };
    let y_final = if m <= 2 { y + 1 } else { y };
    (y_final, m, d)
}
