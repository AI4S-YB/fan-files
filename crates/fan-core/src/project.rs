use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub assay_type: Option<String>,
    pub species: Option<String>,
    pub species_confidence: Option<String>,
    pub species_source: Option<String>,
    pub root_dirs: Option<String>,
    pub summary: Option<String>,
    pub source_server: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct ProjectStore {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl ProjectStore {
    pub fn new(conn: Arc<Mutex<rusqlite::Connection>>) -> Self {
        Self { conn }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }

    pub fn insert(&self, name: &str, assay_type: Option<&str>, species: Option<&str>,
                  species_confidence: Option<&str>, root_dirs: Option<&str>,
                  summary: Option<&str>) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Self::now();
        conn.execute(
            "INSERT INTO project (name, assay_type, species, species_confidence, root_dirs, summary, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![name, assay_type, species, species_confidence, root_dirs, summary, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_species(&self, id: i64, species: &str, source: &str, confidence: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE project SET species=?1, species_source=?2, species_confidence=?3, updated_at=?4 WHERE id=?5",
            params![species, source, confidence, Self::now(), id],
        )?;
        Ok(())
    }

    pub fn link_file(&self, project_id: i64, file_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO project_file (project_id, file_id) VALUES (?1, ?2)",
            params![project_id, file_id],
        )?;
        Ok(())
    }

    pub fn add_relation(&self, project_a: i64, project_b: i64, rel_type: &str, score: f64, reason: Option<&str>) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO project_relation (project_a_id, project_b_id, relation_type, score, reason)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_a, project_b, rel_type, score, reason],
        )?;
        Ok(())
    }

    pub fn get_by_name(&self, name: &str) -> rusqlite::Result<Option<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, assay_type, species, species_confidence, species_source, root_dirs, summary, source_server, created_at, updated_at
             FROM project WHERE name=?1"
        )?;
        let mut rows = stmt.query_map(params![name], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn all(&self) -> rusqlite::Result<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, assay_type, species, species_confidence, species_source, root_dirs, summary, source_server, created_at, updated_at
             FROM project ORDER BY id"
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    pub fn get(&self, id: i64) -> rusqlite::Result<Option<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, assay_type, species, species_confidence, species_source, root_dirs, summary, source_server, created_at, updated_at
             FROM project WHERE id=?1"
        )?;
        let mut rows = stmt.query_map(params![id], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            assay_type: row.get(2)?,
            species: row.get(3)?,
            species_confidence: row.get(4)?,
            species_source: row.get(5)?,
            root_dirs: row.get(6)?,
            summary: row.get(7)?,
            source_server: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    pub fn file_count(&self, project_id: i64) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM project_file WHERE project_id=?1",
            rusqlite::params![project_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c as usize)
    }

    pub fn get_relations(&self, project_id: i64) -> rusqlite::Result<Vec<(String, String, f64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT CASE WHEN pr.project_a_id=?1 THEN p2.name ELSE p1.name END,
                    pr.relation_type, pr.score
             FROM project_relation pr
             JOIN project p1 ON p1.id=pr.project_a_id
             JOIN project p2 ON p2.id=pr.project_b_id
             WHERE pr.project_a_id=?1 OR pr.project_b_id=?1"
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect()
    }
}
