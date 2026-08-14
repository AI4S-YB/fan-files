use crate::{
    config::Settings,
    db::Database,
    models::{Facets, Stats},
};
use std::{sync::Mutex, time::Instant};

pub struct Cache<T> {
    pub loaded: Instant,
    pub value: T,
}
pub struct AppState {
    pub db: Database,
    pub settings: Settings,
    pub stats: Mutex<Option<Cache<Stats>>>,
    pub facets: Mutex<Option<Cache<Facets>>>,
}
impl AppState {
    pub fn new(settings: Settings) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::open(&settings)?;
        Ok(Self {
            db,
            settings,
            stats: Mutex::new(None),
            facets: Mutex::new(None),
        })
    }
}
