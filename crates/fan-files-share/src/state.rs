use crate::{
    config::Settings,
    db::Database,
    models::{Facets, Stats},
};
use fan_core::index::tantivy::TantivyIndex;
use std::{path::PathBuf, sync::Mutex, time::Instant};

pub struct Cache<T> {
    pub loaded: Instant,
    pub value: T,
}
pub struct AppState {
    pub db: Database,
    pub settings: Settings,
    /// Full-text index, shared data dir with the SQLite db
    /// (`<data_dir>/tantivy`). None when the index does not exist yet.
    pub tantivy: Option<TantivyIndex>,
    pub stats: Mutex<Option<Cache<Stats>>>,
    pub facets: Mutex<Option<Cache<Facets>>>,
}
impl AppState {
    pub fn new(settings: Settings) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::open(&settings)?;
        // tantivy dir sits next to the sqlite database file
        let data_dir: PathBuf = settings.database.parent().unwrap().to_path_buf();
        // TantivyIndex::open() appends "tantivy" to data_dir and
        // open_or_create()s the index, so only open an existing one.
        let tantivy = if data_dir.join("tantivy").exists() {
            TantivyIndex::open(&data_dir, true).ok()
        } else {
            None
        };
        Ok(Self {
            db,
            settings,
            tantivy,
            stats: Mutex::new(None),
            facets: Mutex::new(None),
        })
    }
}
