use crate::types::{FileEntry, IndexStatus, RawFileInfo};
use fan_plugin_sdk::{BioMetadata, FormatInfo};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct SqliteStore {
    pub conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open(data_dir: &Path) -> rusqlite::Result<Self> {
        std::fs::create_dir_all(data_dir).ok();
        let conn = Connection::open(data_dir.join("index.db"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT UNIQUE NOT NULL,
                size INTEGER NOT NULL,
                mtime_secs INTEGER NOT NULL,
                hash_sha256 TEXT,
                magic_bytes BLOB,
                mime_type TEXT,
                format_info_json TEXT,
                bio_metadata_json TEXT,
                indexed_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_deleted ON files(deleted);
            CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime_secs);
            CREATE TABLE IF NOT EXISTS tags (
                file_id INTEGER NOT NULL REFERENCES files(id),
                tag TEXT NOT NULL,
                UNIQUE(file_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
            CREATE TABLE IF NOT EXISTS embeddings (
                file_id INTEGER PRIMARY KEY REFERENCES files(id),
                vector BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS relations (
                file_a_id INTEGER NOT NULL REFERENCES files(id),
                file_b_id INTEGER NOT NULL REFERENCES files(id),
                relation_type TEXT NOT NULL,
                score REAL NOT NULL DEFAULT 0.0,
                UNIQUE(file_a_id, file_b_id, relation_type)
            );
            CREATE TABLE IF NOT EXISTS project (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                assay_type TEXT,
                species TEXT,
                species_confidence TEXT,
                species_source TEXT,
                root_dirs TEXT,
                summary TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS project_file (
                project_id INTEGER NOT NULL REFERENCES project(id),
                file_id INTEGER NOT NULL REFERENCES files(id),
                PRIMARY KEY (project_id, file_id)
            );
            CREATE TABLE IF NOT EXISTS project_relation (
                project_a_id INTEGER NOT NULL REFERENCES project(id),
                project_b_id INTEGER NOT NULL REFERENCES project(id),
                relation_type TEXT NOT NULL,
                score REAL NOT NULL DEFAULT 0.0,
                reason TEXT,
                PRIMARY KEY (project_a_id, project_b_id, relation_type)
            );",
        )?;
        Ok(())
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<FileEntry> {
        Ok(FileEntry {
            id: row.get(0)?,
            path: row.get::<_, String>(1)?.into(),
            size: row.get::<_, i64>(2)? as u64,
            mtime_secs: row.get(3)?,
            hash_sha256: row.get(4)?,
            magic_bytes: row.get(5)?,
            mime_type: row.get(6)?,
            format_info: row
                .get::<_, Option<String>>(7)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            bio_metadata: row
                .get::<_, Option<String>>(8)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            indexed_at: row.get(9)?,
            updated_at: row.get(10)?,
            deleted: row.get::<_, i32>(11)? != 0,
        })
    }

    pub fn upsert(
        &self,
        info: &RawFileInfo,
        format_info: Option<&FormatInfo>,
    ) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Self::now();
        let fi_json = format_info.map(|f| serde_json::to_string(f).unwrap());
        conn.execute(
            "INSERT INTO files (path, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, indexed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(path) DO UPDATE SET
                size=excluded.size, mtime_secs=excluded.mtime_secs,
                hash_sha256=excluded.hash_sha256, magic_bytes=excluded.magic_bytes,
                mime_type=excluded.mime_type, format_info_json=excluded.format_info_json,
                updated_at=excluded.updated_at, deleted=0",
            params![
                info.path.to_string_lossy(),
                info.size as i64,
                info.mtime_secs,
                info.hash_sha256,
                info.magic_bytes,
                info.mime_type,
                fi_json,
                now,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_bio_metadata(&self, file_id: i64, meta: &BioMetadata) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let json = serde_json::to_string(meta).unwrap();
        let now = Self::now();
        conn.execute(
            "UPDATE files SET bio_metadata_json=?1, updated_at=?2 WHERE id=?3",
            params![json, now, file_id],
        )?;
        conn.execute("DELETE FROM tags WHERE file_id=?1", params![file_id])?;
        for tag in &meta.tags {
            conn.execute(
                "INSERT OR IGNORE INTO tags (file_id, tag) VALUES (?1, ?2)",
                params![file_id, tag],
            )?;
        }
        Ok(())
    }

    pub fn mark_deleted(&self, path: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET deleted=1, updated_at=?1 WHERE path=?2",
            params![Self::now(), path.to_string_lossy()],
        )?;
        Ok(())
    }

    pub fn purge_old_deleted(&self, keep_days: u32) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let cutoff = Self::now() - (keep_days as i64 * 86400);
        Ok(conn.execute(
            "DELETE FROM files WHERE deleted=1 AND updated_at < ?1",
            params![cutoff],
        )?)
    }

    pub fn get_by_path(&self, path: &Path) -> rusqlite::Result<Option<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE path=?1",
        )?;
        let mut rows = stmt.query_map(params![path.to_string_lossy()], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_by_id(&self, id: i64) -> rusqlite::Result<Option<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE id=?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_by_tag(&self, tag: &str, limit: usize) -> rusqlite::Result<Vec<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, f.size, f.mtime_secs, f.hash_sha256, f.magic_bytes, \
             f.mime_type, f.format_info_json, f.bio_metadata_json, f.indexed_at, \
             f.updated_at, f.deleted
             FROM files f JOIN tags t ON f.id = t.file_id
             WHERE t.tag=?1 AND f.deleted=0 LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![tag, limit as i64], Self::map_row)?;
        rows.collect()
    }

    pub fn all_paths(&self) -> rusqlite::Result<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, path, mtime_secs FROM files WHERE deleted=0")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get::<_, String>(1)?, row.get(2)?))
        })?;
        rows.collect()
    }

    /// Fallback search: LIKE query on path + bio_metadata_json + format_info_json.
    /// Returns (file_id, relevance_score) pairs.
    pub fn search_by_metadata(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<(i64, i32)>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id,
                    (CASE WHEN path LIKE ?1 THEN 3 ELSE 0 END +
                     CASE WHEN bio_metadata_json LIKE ?1 THEN 2 ELSE 0 END +
                     CASE WHEN format_info_json LIKE ?1 THEN 1 ELSE 0 END) as score
             FROM files WHERE deleted=0 AND (
                 path LIKE ?1 OR bio_metadata_json LIKE ?1 OR format_info_json LIKE ?1
             )
             ORDER BY score DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        rows.collect()
    }

    pub fn store_embedding(&self, file_id: i64, vector: &[f32]) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute(
            "INSERT OR REPLACE INTO embeddings (file_id, vector) VALUES (?1, ?2)",
            params![file_id, bytes],
        )?;
        Ok(())
    }

    pub fn load_embeddings(&self) -> rusqlite::Result<Vec<(i64, Vec<f32>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT file_id, vector FROM embeddings")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            let floats: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            Ok((id, floats))
        })?;
        rows.collect()
    }

    pub fn status(&self) -> rusqlite::Result<IndexStatus> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let indexed: i64 =
            conn.query_row("SELECT COUNT(*) FROM files WHERE deleted=0", [], |r| r.get(0))?;
        let deleted: i64 =
            conn.query_row("SELECT COUNT(*) FROM files WHERE deleted=1", [], |r| r.get(0))?;
        let last_scan: Option<i64> =
            conn.query_row("SELECT MAX(indexed_at) FROM files", [], |r| r.get(0))?;
        let last_change: Option<i64> =
            conn.query_row("SELECT MAX(updated_at) FROM files", [], |r| r.get(0))?;
        Ok(IndexStatus {
            total_files: total as u64,
            indexed_files: indexed as u64,
            deleted_files: deleted as u64,
            last_full_scan: last_scan,
            last_change: last_change,
            db_size_bytes: 0,
        })
    }

    /// Attach external public database and search it.
    /// Returns (accession, organism_name, project_title) tuples.
    pub fn search_public(
        &self,
        db_path: &str,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(&format!("ATTACH DATABASE '{}' AS public_sra", db_path))?;
        // Use FTS5 for fast full-text search
        let fts_query = query.split_whitespace()
            .map(|w| format!("\"{}\"", w))
            .collect::<Vec<_>>()
            .join(" ");
        let mut stmt = conn.prepare(
            "SELECT accession, organism_name, project_title
             FROM public_sra.sra_entries
             WHERE rowid IN (SELECT rowid FROM public_sra.sra_fts WHERE sra_fts MATCH ?1)
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect()
    }
}
