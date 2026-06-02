use fan_core::config::Config;
use fan_core::index::sqlite::SqliteStore;
use fan_core::project::ProjectStore;
use std::sync::Arc;

pub fn run(_config: &Config, show_name: Option<&str>) {
    let data_dir = fan_core::config::dirs_fan().join("data");
    let sqlite = match SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };
    let store = ProjectStore::new(Arc::clone(&sqlite.conn));

    match show_name {
        Some(name) => show(&store, name),
        None => list(&store),
    }
}

fn list(store: &ProjectStore) {
    match store.all() {
        Ok(projects) => {
            if projects.is_empty() {
                println!("No projects found. Run 'fan-files infer' first.");
                return;
            }
            for p in &projects {
                let species = p.species.as_deref().unwrap_or("?");
                let conf = p.species_confidence.as_deref().unwrap_or("?");
                let assay = p.assay_type.as_deref().unwrap_or("?");
                let file_count = store.file_count(p.id).unwrap_or(0);
                println!(
                    "{:<25} {:<22} {:<18} {} files",
                    truncate(&p.name, 25),
                    assay,
                    format!("{} ({})", species, conf),
                    file_count,
                );
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn show(store: &ProjectStore, name: &str) {
    match store.get_by_name(name) {
        Ok(Some(p)) => {
            println!("Project: {}", p.name);
            if let Some(ref at) = p.assay_type {
                println!("  Assay:       {}", at);
            }
            println!(
                "  Species:     {} (confidence: {}, source: {})",
                p.species.as_deref().unwrap_or("?"),
                p.species_confidence.as_deref().unwrap_or("?"),
                p.species_source.as_deref().unwrap_or("llm"),
            );
            if let Some(ref dirs) = p.root_dirs {
                if let Ok(parsed) = serde_json::from_str::<Vec<String>>(dirs) {
                    println!("  Directories:");
                    for d in &parsed {
                        println!("    {}", d);
                    }
                }
            }
            println!("  Files:       {}", store.file_count(p.id).unwrap_or(0));
            if let Some(ref s) = p.summary {
                println!("  Summary:     {}", s);
            }
            if let Ok(rels) = store.get_relations(p.id) {
                if !rels.is_empty() {
                    println!("  Relations:");
                    for (other_name, rel_type, score) in &rels {
                        println!("    → {} ({}, score: {:.1})", other_name, rel_type, score);
                    }
                }
            }
        }
        Ok(None) => eprintln!("Project '{}' not found.", name),
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max {
        &s[..max]
    } else {
        s
    }
}

pub fn run_update(
    _config: &Config,
    name: &str,
    species: Option<&str>,
    confidence: Option<&str>,
    assay_type: Option<&str>,
) {
    let data_dir = fan_core::config::dirs_fan().join("data");
    let sqlite = match SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };
    let store = ProjectStore::new(Arc::clone(&sqlite.conn));

    match store.get_by_name(name) {
        Ok(Some(proj)) => {
            let mut updated = false;
            if let Some(sp) = species {
                store
                    .update_species(proj.id, sp, "manual", confidence.unwrap_or("high"))
                    .ok();
                println!(
                    "Updated {}: species={}, confidence={}",
                    name,
                    sp,
                    confidence.unwrap_or("high")
                );
                updated = true;
            }
            if assay_type.is_some() {
                if updated {
                    eprintln!("Note: assay_type update not yet implemented (use species + confidence for now)");
                } else {
                    eprintln!("assay_type update not yet implemented (use species + confidence for now)");
                }
            }
        }
        Ok(None) => eprintln!("Project '{}' not found.", name),
        Err(e) => eprintln!("Error: {}", e),
    }
}
