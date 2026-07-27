use fan_core::config::{Config, DataLayer};
use fan_core::index::IndexEngine;

pub fn list(config: &Config, layer: &DataLayer) {
    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    let index = match IndexEngine::open_at(&data_dir, config, true) {
        Ok(i) => i,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };
    
    let conn = index.sqlite.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, created_at, trigger, summary FROM infer_snapshot ORDER BY id DESC LIMIT 20"
    ).unwrap();
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
    }).unwrap();
    
    println!("{:<6} {:<20} {:<12} {}", "ID", "Time", "Trigger", "Summary");
    for row in rows {
        if let Ok((id, ts, trigger, summary)) = row {
            let time = chrono_str(ts);
            println!("{:<6} {:<20} {:<12} {}", id, time, trigger, summary);
        }
    }
}

pub fn diff(config: &Config, layer: &DataLayer, id1: i64, id2: i64) {
    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    let index = match IndexEngine::open_at(&data_dir, config, true) {
        Ok(i) => i,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };
    
    let conn = index.sqlite.conn.lock().unwrap();
    
    // Count datasets per snapshot
    let count = |sid: i64| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM dataset WHERE snapshot_id = ?1",
            rusqlite::params![sid],
            |r| r.get(0),
        ).unwrap_or(0)
    };
    
    let c1 = count(id1);
    let c2 = count(id2);
    
    println!("Snapshot diff: #{} ({}) vs #{} ({})", id1, c1, id2, c2);
    println!("  Datasets: {} → {} ({:+})", c1, c2, c2 - c1);
    
    // Find datasets in id2 not in id1 (added)
    let mut stmt = conn.prepare(
        "SELECT name FROM dataset WHERE snapshot_id = ?2 AND name NOT IN (SELECT name FROM dataset WHERE snapshot_id = ?1)"
    ).unwrap();
    let added: Vec<String> = stmt.query_map(rusqlite::params![id1, id2], |r| r.get(0))
        .unwrap().filter_map(|x| x.ok()).collect();
    
    if !added.is_empty() {
        println!("  Added ({}):", added.len());
        for n in added.iter().take(10) { println!("    + {}", n); }
        if added.len() > 10 { println!("    ... +{} more", added.len()-10); }
    }
    
    // Find datasets in id1 not in id2 (removed)
    let mut stmt = conn.prepare(
        "SELECT name FROM dataset WHERE snapshot_id = ?1 AND name NOT IN (SELECT name FROM dataset WHERE snapshot_id = ?2)"
    ).unwrap();
    let removed: Vec<String> = stmt.query_map(rusqlite::params![id1, id2], |r| r.get(0))
        .unwrap().filter_map(|x| x.ok()).collect();
    
    if !removed.is_empty() {
        println!("  Removed ({}):", removed.len());
        for n in removed.iter().take(10) { println!("    - {}", n); }
        if removed.len() > 10 { println!("    ... -{} more", removed.len()-10); }
    }
    
    if added.is_empty() && removed.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT d1.name FROM dataset d1 JOIN dataset d2 ON d1.name = d2.name WHERE d1.snapshot_id = ?1 AND d2.snapshot_id = ?2 AND (d1.dataset_type != d2.dataset_type OR d1.species != d2.species)"
        ).unwrap();
        let modified: Vec<String> = stmt.query_map(rusqlite::params![id1, id2], |r| r.get(0))
            .unwrap().filter_map(|x| x.ok()).collect();
        if !modified.is_empty() {
            println!("  Modified ({}):", modified.len());
            for n in modified.iter().take(10) { println!("    ~ {}", n); }
        }
    }
}

pub fn rollback(config: &Config, layer: &DataLayer, id: i64) {
    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    let index = match IndexEngine::open_at(&data_dir, config, false) {
        Ok(i) => i,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };
    
    match index.sqlite.rollback_to_snapshot(id) {
        Ok(_) => println!("Rolled back to snapshot #{}", id),
        Err(e) => eprintln!("Rollback failed: {}", e),
    }
}

fn chrono_str(ts: i64) -> String {
    // Simple unix timestamp to readable
    let secs = ts;
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    format!("{:02}:{:02}:{:02}", hours, mins, secs % 60)
}
