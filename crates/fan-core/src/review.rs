use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingItem {
    pub project: String,
    pub field: String,
    pub current_value: Option<String>,
    pub confidence: Option<String>,
    pub candidates: Vec<String>,
    pub timestamp: i64,
}

pub struct ReviewStore {
    pub path: PathBuf,
}

impl ReviewStore {
    pub fn new() -> Self {
        Self {
            path: crate::config::dirs_fan().join("pending_review.json"),
        }
    }

    pub fn load(&self) -> Result<Vec<PendingItem>, Box<dyn std::error::Error>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let data = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn save(&self, items: &[PendingItem]) -> Result<(), Box<dyn std::error::Error>> {
        let data = serde_json::to_string_pretty(items)?;
        std::fs::write(&self.path, data)?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&[])
    }

    pub fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }
}
