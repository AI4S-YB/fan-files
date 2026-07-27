//! Record correction and auto-generate rules from patterns

use std::collections::HashMap;
use std::path::PathBuf;

pub fn run(dataset: Option<String>, asset: Option<String>, new_type: Option<String>) {
    let fan_dir = PathBuf::from(
        std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
    ).join(".fan-files");
    
    let path = fan_dir.join("corrections.json");
    let mut data = load_or_init(&path);
    
    if let (Some(ds), Some(asset_name), Some(new_typ)) = (dataset, asset, new_type) {
        // Record a specific correction
        record_correction(&mut data, &ds, &asset_name, &new_typ);
        save(&path, &data);
        eprintln!("Correction recorded: {} / {} → {}", ds, asset_name, new_typ);
    }
    
    // Check for new patterns reaching threshold
    let threshold = data["threshold"].as_u64().unwrap_or(3) as usize;
    let new_rules: Vec<String> = data["patterns"].as_array()
        .map(|arr| arr.iter()
            .filter(|p| p["count"].as_u64().unwrap_or(0) as usize >= threshold)
            .filter_map(|p| p["pattern"].as_str().map(String::from))
            .collect())
        .unwrap_or_default();
    
    if !new_rules.is_empty() {
        eprintln!("\nSystem learned rules (count >= {}):", threshold);
        for r in &new_rules {
            eprintln!("  - {}", r);
        }
        eprintln!("\nRun 'fan-files rules promote' to lock these rules.");
    }
}

fn load_or_init(path: &PathBuf) -> serde_json::Value {
    if path.exists() {
        let s = std::fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&s).unwrap_or_else(|_| default_corrections())
    } else {
        let d = default_corrections();
        let s = serde_json::to_string_pretty(&d).unwrap_or_default();
        std::fs::write(path, s).ok();
        d
    }
}

fn default_corrections() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "threshold": 3,
        "patterns": [],
        "history": []
    })
}

fn record_correction(data: &mut serde_json::Value, dataset: &str, asset: &str, new_type: &str) {
    // Extract pattern from asset name: remove version suffix like _v1.0
    let pattern = extract_pattern(asset);
    
    // Update or create pattern
    let patterns = data["patterns"].as_array_mut()
        .expect("patterns should be array");
    
    let mut found = false;
    for p in patterns.iter_mut() {
        if p["pattern"].as_str() == Some(&format!("{} → {}", pattern, new_type)) {
            let count = p["count"].as_u64().unwrap_or(0) + 1;
            p["count"] = serde_json::json!(count);
            p["last_seen"] = serde_json::json!(chrono_now());
            found = true;
            break;
        }
    }
    if !found {
        patterns.push(serde_json::json!({
            "pattern": format!("{} → {}", pattern, new_type),
            "count": 1,
            "last_seen": chrono_now(),
            "auto_generated": true
        }));
    }
    
    // Add history entry
    let history = data["history"].as_array_mut()
        .expect("history should be array");
    history.push(serde_json::json!({
        "dataset": dataset,
        "asset": asset,
        "new_type": new_type,
        "timestamp": chrono_now()
    }));
}

fn extract_pattern(asset_name: &str) -> String {
    // Remove version suffix: "cds_v1.0" → "cds"
    // Remove assembly prefix: "assembly_v1.0" → "assembly"
    let name = asset_name
        .trim_end_matches(char::is_numeric)
        .trim_end_matches('.')
        .trim_end_matches('_')
        .to_string();
    name
}

fn chrono_now() -> String {
    // Simple timestamp
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

fn save(path: &PathBuf, data: &serde_json::Value) {
    let s = serde_json::to_string_pretty(data).unwrap_or_default();
    std::fs::write(path, s).ok();
}
