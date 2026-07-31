use fan_core::config::{Config, DataLayer};
use fan_core::index;
use fan_core::index::sqlite::SqliteStore;

pub fn run(_config: &Config, layer: &DataLayer, show_name: Option<&str>) {
    let sqlite = match index::open_sqlite(layer) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };

    match show_name {
        Some(name) => show(&sqlite, name),
        None => list(&sqlite),
    }
}

fn list(sqlite: &SqliteStore) {
    match sqlite.all_datasets() {
        Ok(datasets) => {
            if datasets.is_empty() {
                println!("No datasets found. Run 'fan-files discover' first.");
                return;
            }
            println!("{:<40} {:<15} {:<20} {:>6} {:>6}",
                "Dataset", "Type", "Species", "Assets", "Files");
            println!("{}", "-".repeat(90));
            for ds in &datasets {
                let dtype = ds.dataset_type.as_deref().unwrap_or("?");
                let species = ds.species.as_deref().unwrap_or("?");
                let asset_count: i64 = sqlite.conn.lock().unwrap()
                    .query_row("SELECT COUNT(*) FROM asset WHERE dataset_id = ?1",
                        rusqlite::params![ds.id], |r| r.get(0)).unwrap_or(0);
                let file_count: i64 = sqlite.conn.lock().unwrap()
                    .query_row(
                        "SELECT COUNT(DISTINCT af.file_id) FROM asset_file af \
                         JOIN asset a ON af.asset_id = a.id WHERE a.dataset_id = ?1",
                        rusqlite::params![ds.id], |r| r.get(0)).unwrap_or(0);
                println!("{:<40} {:<15} {:<20} {:>6} {:>6}",
                    truncate(&ds.name, 40), dtype, species, asset_count, file_count);
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn show(sqlite: &SqliteStore, name: &str) {
    match sqlite.all_datasets() {
        Ok(datasets) => {
            if let Some(ds) = datasets.iter().find(|d| d.name == name) {
                println!("Dataset: {}", ds.name);
                println!("  Path:        {}", ds.path);
                println!("  Type:        {}", ds.dataset_type.as_deref().unwrap_or("?"));
                if let Some(ref s) = ds.species {
                    println!("  Species:     {}", s);
                }
                if let Some(ref c) = ds.species_confidence {
                    println!("  Confidence:  {}", c);
                }
                if let Some(ref s) = ds.summary {
                    println!("  Summary:     {}", s);
                }
                println!();

                // List assets
                let conn = sqlite.conn.lock().unwrap();
                let mut stmt = conn.prepare(
                    "SELECT a.id, a.name, a.asset_type, COUNT(af.file_id) \
                     FROM asset a LEFT JOIN asset_file af ON a.id = af.asset_id \
                     WHERE a.dataset_id = ?1 GROUP BY a.id ORDER BY a.id"
                ).unwrap();
                let assets: Vec<(i64, Option<String>, Option<String>, i64)> = stmt
                    .query_map(rusqlite::params![ds.id], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    }).unwrap().filter_map(|r| r.ok()).collect();

                println!("  Assets ({}):", assets.len());
                for (_, a_name, a_type, file_count) in &assets {
                    println!("    [{:<20}] {:>4} files  {}",
                        a_type.as_deref().unwrap_or("?"),
                        file_count,
                        a_name.as_deref().unwrap_or(""));
                }
            } else {
                eprintln!("Dataset '{}' not found", name);
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}...", &s[..max.saturating_sub(3)]) }
}
