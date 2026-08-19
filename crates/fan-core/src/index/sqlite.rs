use crate::types::{FileEntry, IndexStatus, RawFileInfo};
use fan_plugin_sdk::{BioMetadata, FormatInfo};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct SqliteStore {
    pub conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open(data_dir: &Path) -> rusqlite::Result<Self> {
        std::fs::create_dir_all(data_dir).ok();
        let conn = Connection::open(data_dir.join("index.db"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-64000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_read_only(data_dir: &Path) -> rusqlite::Result<Self> {
        let db_path = data_dir.join("index.db");
        // Do not use `immutable=1` here. The daemon writes in WAL mode, and an
        // immutable connection ignores the WAL file, so live readers can miss
        // both schema changes and freshly indexed rows until a checkpoint.
        let uri = format!("file:{}?mode=ro", db_path.to_string_lossy());
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        conn.execute_batch(
            "PRAGMA query_only=ON;
             PRAGMA cache_size=-64000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Begin an explicit transaction for batch writes.
    pub fn begin_batch(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE;")
    }

    /// Commit the current batch transaction.
    pub fn commit_batch(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("COMMIT;")
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

        // v2 migration: source_server tracking
        {
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap_or(0);
            if version < 2 {
                conn.execute_batch(
                    "ALTER TABLE files ADD COLUMN source_server TEXT NOT NULL DEFAULT 'local';
                     ALTER TABLE project ADD COLUMN source_server TEXT DEFAULT 'local';
                     CREATE INDEX IF NOT EXISTS idx_files_server ON files(source_server);
                     PRAGMA user_version = 2;",
                )?;
            }
        }
        // v3: dataset/asset/asset_file tables
        {
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap_or(0);
            if version < 3 {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS dataset (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        name TEXT NOT NULL,
                        path TEXT NOT NULL UNIQUE,
                        dataset_type TEXT,
                        species TEXT,
                        species_confidence TEXT,
                        species_source TEXT,
                        summary TEXT,
                        indexed_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE IF NOT EXISTS asset (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        dataset_id INTEGER NOT NULL REFERENCES dataset(id),
                        name TEXT,
                        asset_type TEXT,
                        path TEXT,
                        indexed_at INTEGER NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_asset_dataset ON asset(dataset_id);
                    CREATE TABLE IF NOT EXISTS asset_file (
                        asset_id INTEGER NOT NULL REFERENCES asset(id),
                        file_id INTEGER NOT NULL REFERENCES files(id),
                        role TEXT,
                        PRIMARY KEY (asset_id, file_id)
                    );
                    CREATE INDEX IF NOT EXISTS idx_asset_file_file ON asset_file(file_id);
                    PRAGMA user_version = 3;",
                )?;
            }
        }

        // v4: infer_snapshot for re-infer version tracking
        {
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap_or(0);
            if version < 4 {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS infer_snapshot (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        created_at INTEGER NOT NULL,
                        trigger TEXT NOT NULL,
                        rule_hash TEXT,
                        summary TEXT
                    );
                    ALTER TABLE dataset ADD COLUMN snapshot_id INTEGER REFERENCES infer_snapshot(id);
                    ALTER TABLE asset ADD COLUMN snapshot_id INTEGER REFERENCES infer_snapshot(id);
                    PRAGMA user_version = 4;",
                )?;
            }
        }

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
            source_server: row.get(2)?,
            size: row.get::<_, i64>(3)? as u64,
            mtime_secs: row.get(4)?,
            hash_sha256: row.get(5)?,
            magic_bytes: row.get(6)?,
            mime_type: row.get(7)?,
            format_info: row
                .get::<_, Option<String>>(8)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            bio_metadata: row
                .get::<_, Option<String>>(9)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            indexed_at: row.get(10)?,
            updated_at: row.get(11)?,
            deleted: row.get::<_, i32>(12)? != 0,
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
            "INSERT INTO files (path, source_server, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, indexed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(path) DO UPDATE SET
                source_server=excluded.source_server,
                size=excluded.size, mtime_secs=excluded.mtime_secs,
                hash_sha256=excluded.hash_sha256, magic_bytes=excluded.magic_bytes,
                mime_type=excluded.mime_type, format_info_json=excluded.format_info_json,
                updated_at=excluded.updated_at, deleted=0",
            params![
                info.path.to_string_lossy(),
                info.source_server,
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
            "SELECT id, path, source_server, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE path=?1",
        )?;
        let mut rows = stmt.query_map(params![path.to_string_lossy()], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_by_id(&self, id: i64) -> rusqlite::Result<Option<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, source_server, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE id=?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_by_tag(&self, tag: &str, limit: usize) -> rusqlite::Result<Vec<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, f.source_server, f.size, f.mtime_secs, f.hash_sha256, f.magic_bytes, \
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
    pub fn search_by_metadata(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(i64, i32)>> {
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
             ORDER BY score DESC LIMIT ?2",
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

    /// Return embeddings only for the supplied full-text candidates.
    ///
    /// Search used to deserialize every embedding in the database for every
    /// query. Restricting the lookup to the (at most 50) Tantivy candidates
    /// keeps the interactive path bounded and avoids loading the model when
    /// none of those candidates has an embedding.
    pub fn load_embeddings_for_ids(
        &self,
        file_ids: &[i64],
    ) -> rusqlite::Result<Vec<(i64, Vec<f32>)>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().unwrap();
        let placeholders = std::iter::repeat_n("?", file_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT file_id, vector FROM embeddings WHERE file_id IN ({})",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(file_ids.iter()), |row| {
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

    pub fn count_with_bio_metadata(&self) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM files WHERE bio_metadata_json IS NOT NULL AND bio_metadata_json != '' AND deleted=0",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c as u64)
    }

    pub fn search_by_server(
        &self,
        server: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, mtime_secs FROM files WHERE source_server=?1 AND deleted=0 LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![server, limit as i64], |row| {
            Ok((row.get(0)?, row.get::<_, String>(1)?, row.get(2)?))
        })?;
        rows.collect()
    }

    pub fn status(&self) -> rusqlite::Result<IndexStatus> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let indexed: i64 =
            conn.query_row("SELECT COUNT(*) FROM files WHERE deleted=0", [], |r| {
                r.get(0)
            })?;
        let deleted: i64 =
            conn.query_row("SELECT COUNT(*) FROM files WHERE deleted=1", [], |r| {
                r.get(0)
            })?;
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
            servers: vec![],
        })
    }

    pub fn status_by_server(&self) -> rusqlite::Result<Vec<crate::types::ServerStats>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT source_server, COUNT(*) as cnt, MAX(indexed_at)
             FROM files WHERE deleted=0
             GROUP BY source_server ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::ServerStats {
                server: row.get(0)?,
                file_count: row.get::<_, i64>(1)? as u64,
                last_scan: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    // ═══ v2: Dataset/Asset CRUD ═══

    pub fn insert_dataset(
        &self,
        name: &str,
        path: &str,
        dataset_type: Option<&str>,
        species: Option<&str>,
        confidence: Option<&str>,
        summary: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        // UPSERT: insert new or update existing (keeps original indexed_at)
        let id: i64 = conn.query_row(
            "INSERT INTO dataset (name, path, dataset_type, species, species_confidence, summary, indexed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET
                 name=excluded.name,
                 dataset_type=excluded.dataset_type,
                 species=excluded.species,
                 species_confidence=excluded.species_confidence,
                 summary=excluded.summary,
                 updated_at=excluded.updated_at
             RETURNING id",
            rusqlite::params![name, path, dataset_type, species, confidence, summary, now, now],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn insert_asset(
        &self,
        dataset_id: i64,
        name: Option<&str>,
        asset_type: Option<&str>,
        path: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO asset (dataset_id, name, asset_type, path, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![dataset_id, name, asset_type, path, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn link_asset_file(
        &self,
        asset_id: i64,
        file_id: i64,
        role: Option<&str>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO asset_file (asset_id, file_id, role) VALUES (?1, ?2, ?3)",
            rusqlite::params![asset_id, file_id, role],
        )?;
        Ok(())
    }

    /// Batch-link files under one auxiliary_files asset in a single transaction.
    /// Creates the asset automatically. Avoids per-file lock overhead at scale.
    pub fn link_auxiliary_batch(
        &self,
        ds_id: i64,
        file_ids: &[i64],
        role: &str,
    ) -> rusqlite::Result<u64> {
        let mut conn = self.conn.lock().unwrap();
        let now = Self::now();
        conn.execute(
            "INSERT INTO asset (dataset_id, name, asset_type, indexed_at) VALUES (?1, 'auxiliary_files', 'other', ?2)",
            rusqlite::params![ds_id, now],
        )?;
        let a_id = conn.last_insert_rowid();
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO asset_file (asset_id, file_id, role) VALUES (?1, ?2, ?3)",
        )?;
        for fid in file_ids {
            stmt.execute(rusqlite::params![a_id, fid, role])?;
        }
        Ok(file_ids.len() as u64)
    }

    pub fn all_datasets(&self) -> rusqlite::Result<Vec<crate::types::DatasetEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, path, dataset_type, species, species_confidence, species_source, summary, indexed_at, updated_at FROM dataset ORDER BY id"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::DatasetEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                dataset_type: row.get(3)?,
                species: row.get(4)?,
                species_confidence: row.get(5)?,
                species_source: row.get(6)?,
                summary: row.get(7)?,
                indexed_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })?;
        rows.collect()
    }

    /// Find the most specific dataset that contains the requested path, or a
    /// child dataset when the requested path is a broader project directory.
    pub fn find_dataset_for_path(
        &self,
        path: &str,
    ) -> rusqlite::Result<Option<crate::types::DatasetEntry>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, path, dataset_type, species, species_confidence,
                    species_source, summary, indexed_at, updated_at
             FROM dataset
             WHERE path=?1
                OR (length(?1)>length(path)
                    AND substr(?1, 1, length(path)+1)=path || '/')
                OR (length(path)>length(?1)
                    AND substr(path, 1, length(?1)+1)=?1 || '/')
             ORDER BY CASE
                        WHEN path=?1 THEN 0
                        WHEN length(?1)>length(path)
                         AND substr(?1, 1, length(path)+1)=path || '/' THEN 1
                        ELSE 2
                      END,
                      abs(length(path) - length(?1))
             LIMIT 1",
            rusqlite::params![path.trim_end_matches('/')],
            |row| {
                Ok(crate::types::DatasetEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    dataset_type: row.get(3)?,
                    species: row.get(4)?,
                    species_confidence: row.get(5)?,
                    species_source: row.get(6)?,
                    summary: row.get(7)?,
                    indexed_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            },
        )
        .optional()
    }

    /// Return a bounded set of candidate datasets for a species. The dataset
    /// table is the authoritative inferred-metadata layer and is several
    /// orders of magnitude smaller than the files table.
    pub fn datasets_by_species(
        &self,
        species: &str,
        exclude_id: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<crate::types::DatasetEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, path, dataset_type, species, species_confidence,
                    species_source, summary, indexed_at, updated_at
             FROM dataset
             WHERE id != ?1 AND species = ?2 COLLATE NOCASE
             ORDER BY updated_at DESC, id
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![exclude_id, species, limit as i64],
            |row| {
                Ok(crate::types::DatasetEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    dataset_type: row.get(3)?,
                    species: row.get(4)?,
                    species_confidence: row.get(5)?,
                    species_source: row.get(6)?,
                    summary: row.get(7)?,
                    indexed_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            },
        )?;
        rows.collect()
    }

    /// Read a bounded page of data needed to rebuild the full-text index.
    /// This avoids holding millions of paths and issuing one SQLite query per
    /// file during a rebuild.
    pub fn index_documents_after(
        &self,
        after_id: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<(i64, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, format_info_json, bio_metadata_json
             FROM files
             WHERE deleted=0 AND id>?1
             ORDER BY id
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![after_id, limit as i64], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let format_json: Option<String> = row.get(2)?;
            let bio_json: Option<String> = row.get(3)?;

            let format_text = format_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<FormatInfo>(raw).ok())
                .map(|format| format!("{:?}", format))
                .unwrap_or_default();
            let bio = bio_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<BioMetadata>(raw).ok());
            let species = bio
                .as_ref()
                .and_then(|metadata| metadata.species.as_deref())
                .unwrap_or("");
            let assay = bio
                .as_ref()
                .and_then(|metadata| metadata.assay_type.as_deref())
                .unwrap_or("");
            let metadata_text = format!("{} {} {} {}", path, species, assay, format_text);
            Ok((id, path, metadata_text))
        })?;
        rows.collect()
    }

    pub fn count_assets(&self, dataset_id: i64) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM asset WHERE dataset_id = ?1",
            rusqlite::params![dataset_id],
            |r| r.get(0),
        )
    }

    pub fn count_dataset_files(&self, dataset_id: i64) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM asset_file af JOIN asset a ON af.asset_id = a.id WHERE a.dataset_id = ?1",
            rusqlite::params![dataset_id],
            |r| r.get(0),
        )
    }

    // ═══ Snapshot CRUD (re-infer version tracking) ═══

    pub fn create_snapshot(
        &self,
        trigger: &str,
        rule_hash: &str,
        summary: &str,
    ) -> rusqlite::Result<i64> {
        let now = Self::now();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO infer_snapshot (created_at, trigger, rule_hash, summary) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![now, trigger, rule_hash, summary],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn set_dataset_snapshot(&self, dataset_id: i64, snapshot_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE dataset SET snapshot_id = ?1 WHERE id = ?2",
            rusqlite::params![snapshot_id, dataset_id],
        )?;
        Ok(())
    }

    pub fn latest_snapshot(&self) -> rusqlite::Result<Option<(i64, String)>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT id, summary FROM infer_snapshot ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn rollback_to_snapshot(&self, snapshot_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        // Delete datasets and assets created AFTER this snapshot
        conn.execute("DELETE FROM asset_file WHERE asset_id IN (SELECT id FROM asset WHERE snapshot_id > ?1)", rusqlite::params![snapshot_id])?;
        conn.execute(
            "DELETE FROM asset WHERE snapshot_id > ?1",
            rusqlite::params![snapshot_id],
        )?;
        conn.execute(
            "DELETE FROM dataset WHERE snapshot_id > ?1",
            rusqlite::params![snapshot_id],
        )?;
        Ok(())
    }
}
