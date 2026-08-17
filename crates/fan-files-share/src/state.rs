use crate::{
    config::Settings,
    db::Database,
    models::{Facets, Stats},
};
use fan_core::index::tantivy::TantivyIndex;
use std::{sync::Mutex, time::Instant};

pub struct Cache<T> {
    pub loaded: Instant,
    pub value: T,
}
pub struct AppState {
    pub db: Database,
    pub settings: Settings,
    /// Full-text index, shared data dir with the SQLite db
    /// (`<data_dir>/tantivy`). Lazily opened on the first search so that a
    /// share started before the CLI built the index picks it up without a
    /// restart; None when the index does not exist yet.
    pub tantivy: Mutex<Option<TantivyIndex>>,
    pub stats: Mutex<Option<Cache<Stats>>>,
    pub facets: Mutex<Option<Cache<Facets>>>,
}
impl AppState {
    pub fn new(settings: Settings) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::open(&settings)?;
        Ok(Self {
            db,
            settings,
            tantivy: Mutex::new(None),
            stats: Mutex::new(None),
            facets: Mutex::new(None),
        })
    }
}
