use fan_core::config::{Config, DataLayer};
use fan_core::index::sqlite::SqliteStore;
use fan_core::index::tantivy::TantivyIndex;
use std::path::{Path, PathBuf};

const BATCH_SIZE: usize = 20_000;

pub fn run(_config: &Config, layer: &DataLayer) {
    if let Err(error) = rebuild(layer) {
        eprintln!("Failed to rebuild Tantivy index: {error}");
    }
}

fn rebuild(layer: &DataLayer) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = match layer {
        DataLayer::User => fan_core::config::dirs_fan().join("data"),
        DataLayer::Global => fan_core::config::dirs_fan_global().join("data"),
    };
    let sqlite = SqliteStore::open_read_only(&data_dir)?;
    let expected = sqlite.status()?.indexed_files;
    let rebuild_root = data_dir.join(format!(".tantivy-rebuild-{}", std::process::id()));
    if rebuild_root.exists() {
        return Err(format!(
            "temporary rebuild directory already exists: {}",
            rebuild_root.display()
        )
        .into());
    }

    eprintln!("Rebuilding Tantivy index for {expected} files...");
    let result = build_fresh_index(&sqlite, &rebuild_root, expected);
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&rebuild_root);
        return Err(error);
    }

    let fresh_dir = rebuild_root.join("tantivy");
    let live_dir = data_dir.join("tantivy");
    replace_index(&live_dir, &fresh_dir)?;
    let _ = std::fs::remove_dir(&rebuild_root);
    eprintln!("Tantivy index rebuilt: {expected} unique files indexed");
    Ok(())
}

fn build_fresh_index(
    sqlite: &SqliteStore,
    rebuild_root: &Path,
    expected: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(rebuild_root)?;
    let index = TantivyIndex::open(rebuild_root, false)?;
    let mut after_id = 0i64;
    let mut indexed = 0u64;

    loop {
        let documents = sqlite.index_documents_after(after_id, BATCH_SIZE)?;
        if documents.is_empty() {
            break;
        }
        for (id, path, metadata) in &documents {
            index.index_file(*id, Path::new(path), metadata, &[])?;
            after_id = *id;
            indexed += 1;
        }
        index.commit()?;
        eprintln!("  Progress: {indexed}/{expected} files");
    }

    let actual = index.num_docs()?;
    if indexed != expected || actual != expected {
        return Err(format!(
            "rebuilt index validation failed: SQLite={expected}, processed={indexed}, Tantivy={actual}"
        )
        .into());
    }
    drop(index);
    Ok(())
}

fn replace_index(live_dir: &Path, fresh_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let previous = sibling_path(
        live_dir,
        &format!("tantivy.previous-{}", std::process::id()),
    );
    if previous.exists() {
        return Err(format!("refusing to overwrite backup index: {}", previous.display()).into());
    }

    if live_dir.exists() {
        std::fs::rename(live_dir, &previous)?;
    }
    if let Err(error) = std::fs::rename(fresh_dir, live_dir) {
        if previous.exists() {
            let _ = std::fs::rename(&previous, live_dir);
        }
        return Err(error.into());
    }
    if previous.exists() {
        std::fs::remove_dir_all(previous)?;
    }
    Ok(())
}

fn sibling_path(path: &Path, name: &str) -> PathBuf {
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}
