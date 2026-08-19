use crate::{config::Settings, error::AppError, models::*};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OpenFlags, OptionalExtension, params};
use std::{path::Path, time::Duration};

const RELEVANCE_CURSOR_FACTOR: i64 = 1_000_000_000;

#[derive(Clone)]
pub struct Database {
    pool: Pool<SqliteConnectionManager>,
}

impl Database {
    pub fn open(settings: &Settings) -> Result<Self, Box<dyn std::error::Error>> {
        if !settings.database.is_file() {
            return Err(format!("database does not exist: {}", settings.database.display()).into());
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let timeout = Duration::from_millis(settings.busy_timeout_ms);
        let manager = SqliteConnectionManager::file(&settings.database)
            .with_flags(flags)
            .with_init(move |conn| {
                conn.busy_timeout(timeout)?;
                conn.pragma_update(None, "query_only", true)?;
                Ok(())
            });
        let pool = Pool::builder()
            .max_size(settings.pool_size)
            .build(manager)?;
        Ok(Self { pool })
    }

    pub fn readiness(&self, supported: &[i64]) -> Result<i64, AppError> {
        let conn = self.pool.get()?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if !supported.contains(&version) {
            return Err(AppError::NotReady(format!(
                "unsupported schema version {version}"
            )));
        }
        for table in ["dataset", "asset", "asset_file", "files"] {
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(AppError::NotReady(format!("missing table {table}")));
            }
        }
        conn.query_row("SELECT 1 FROM dataset LIMIT 1", [], |_| Ok(()))
            .optional()?;
        Ok(version)
    }

    pub fn datasets(
        &self,
        query: &DatasetQuery,
        limit: u32,
        expose_path: bool,
    ) -> Result<PageEnvelope<DatasetSummary>, AppError> {
        let raw_cursor = decode_cursor(query.cursor)?.unwrap_or(0);
        let q = clean_query(query.q.as_deref())?;
        let pattern = q.as_deref().map(escape_like);
        // GUI-T4: 排序键。向后兼容：带 q 且未显式指定 sort 时仍按相关度排序
        // （旧默认行为）；显式 sort=id|name|file_count 则按列排序。
        let sort_key = query.sort.as_deref().unwrap_or("id");
        let relevance = q.is_some() && (sort_key == "relevance" || query.sort.is_none());
        let conn = self.pool.get()?;
        let type_counts = dataset_type_counts(&conn, query.species.as_deref(), pattern.as_deref())?;
        let (mut data, ranks) = if relevance {
            let cursor_rank = raw_cursor / RELEVANCE_CURSOR_FACTOR;
            let cursor_id = raw_cursor % RELEVANCE_CURSOR_FACTOR;
            let exact = q.as_deref().expect("relevance requires q");
            let prefix = format!("{}%", escape_like_value(exact));
            let mut stmt = conn.prepare(
                "WITH ranked AS (
                    SELECT d.*,
                        CASE
                            WHEN d.name=?4 COLLATE NOCASE THEN 0
                            WHEN d.name LIKE ?5 ESCAPE char(92) COLLATE NOCASE THEN 1
                            WHEN d.name LIKE ?3 ESCAPE char(92) COLLATE NOCASE THEN 2
                            WHEN d.species=?4 COLLATE NOCASE THEN 3
                            WHEN d.species LIKE ?3 ESCAPE char(92) COLLATE NOCASE THEN 4
                            WHEN d.dataset_type LIKE ?3 ESCAPE char(92) COLLATE NOCASE THEN 5
                            ELSE 6
                        END AS relevance_rank
                    FROM dataset d
                    WHERE (?1 IS NULL OR d.species=?1 COLLATE NOCASE)
                      AND (?2 IS NULL OR d.dataset_type=?2 COLLATE NOCASE)
                      AND (d.name LIKE ?3 ESCAPE char(92) COLLATE NOCASE
                           OR d.species LIKE ?3 ESCAPE char(92) COLLATE NOCASE
                           OR d.dataset_type LIKE ?3 ESCAPE char(92) COLLATE NOCASE
                           OR d.summary LIKE ?3 ESCAPE char(92) COLLATE NOCASE)
                )
                SELECT d.id,d.name,d.dataset_type,d.species,d.summary,d.path,d.updated_at,
                       COUNT(DISTINCT a.id),COUNT(DISTINCT af.file_id),d.relevance_rank
                FROM ranked d
                LEFT JOIN asset a ON a.dataset_id=d.id
                LEFT JOIN asset_file af ON af.asset_id=a.id
                WHERE d.relevance_rank>?6 OR (d.relevance_rank=?6 AND d.id>?7)
                GROUP BY d.id ORDER BY d.relevance_rank,d.id LIMIT ?8",
            )?;
            let rows = stmt.query_map(
                params![
                    query.species.as_deref(),
                    query.dataset_type.as_deref(),
                    pattern,
                    exact,
                    prefix,
                    cursor_rank,
                    cursor_id,
                    i64::from(limit + 1)
                ],
                |row| Ok((dataset_summary(row, expose_path)?, row.get::<_, i64>(9)?)),
            )?;
            let pairs: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
            pairs.into_iter().unzip()
        } else if matches!(sort_key, "name" | "file_count") {
            // 名称/文件数排序：游标=已翻过的行数（OFFSET）。排序键变化时前端会
            // 重置分页回第一页，因此 offset 游标在各自排序键下始终正确。
            let order_clause = if sort_key == "name" {
                "d.name COLLATE NOCASE, d.id"
            } else {
                "COUNT(DISTINCT af.file_id), d.id"
            };
            let sql = format!(
                "SELECT d.id,d.name,d.dataset_type,d.species,d.summary,d.path,d.updated_at,
                    COUNT(DISTINCT a.id),COUNT(DISTINCT af.file_id)
                 FROM dataset d
                 LEFT JOIN asset a ON a.dataset_id=d.id
                 LEFT JOIN asset_file af ON af.asset_id=a.id
                 WHERE (?1 IS NULL OR d.species=?1 COLLATE NOCASE)
                   AND (?2 IS NULL OR d.dataset_type=?2 COLLATE NOCASE)
                   AND (?3 IS NULL OR d.name LIKE ?3 ESCAPE char(92) COLLATE NOCASE
                        OR d.species LIKE ?3 ESCAPE char(92) COLLATE NOCASE
                        OR d.dataset_type LIKE ?3 ESCAPE char(92) COLLATE NOCASE
                        OR d.summary LIKE ?3 ESCAPE char(92) COLLATE NOCASE)
                 GROUP BY d.id ORDER BY {order_clause} LIMIT ?4 OFFSET ?5"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![
                    query.species.as_deref(),
                    query.dataset_type.as_deref(),
                    pattern,
                    i64::from(limit + 1),
                    raw_cursor
                ],
                |row| dataset_summary(row, expose_path),
            )?;
            let data: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
            let ranks = vec![0; data.len()];
            (data, ranks)
        } else {
            let mut stmt = conn.prepare(
                "SELECT d.id,d.name,d.dataset_type,d.species,d.summary,d.path,d.updated_at,
                    COUNT(DISTINCT a.id),COUNT(DISTINCT af.file_id)
             FROM dataset d
             LEFT JOIN asset a ON a.dataset_id=d.id
             LEFT JOIN asset_file af ON af.asset_id=a.id
             WHERE d.id>?1
               AND (?2 IS NULL OR d.species=?2 COLLATE NOCASE)
               AND (?3 IS NULL OR d.dataset_type=?3 COLLATE NOCASE)
               AND (?4 IS NULL OR d.name LIKE ?4 ESCAPE char(92) COLLATE NOCASE
                    OR d.species LIKE ?4 ESCAPE char(92) COLLATE NOCASE
                    OR d.dataset_type LIKE ?4 ESCAPE char(92) COLLATE NOCASE
                    OR d.summary LIKE ?4 ESCAPE char(92) COLLATE NOCASE)
             GROUP BY d.id ORDER BY d.id LIMIT ?5",
            )?;
            let rows = stmt.query_map(
                params![
                    raw_cursor,
                    query.species.as_deref(),
                    query.dataset_type.as_deref(),
                    pattern,
                    i64::from(limit + 1)
                ],
                |row| dataset_summary(row, expose_path),
            )?;
            let data: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
            let ranks = vec![0; data.len()];
            (data, ranks)
        };
        let has_more = data.len() > limit as usize;
        let mut ranks = ranks;
        if has_more {
            data.pop();
            ranks.pop();
        }
        let next_cursor = has_more
            .then(|| {
                let item = data.last()?;
                Some(if relevance {
                    ranks.last()? * RELEVANCE_CURSOR_FACTOR + item.id
                } else if sort_key == "name" || sort_key == "file_count" {
                    raw_cursor + i64::from(limit) // 下一页 OFFSET
                } else {
                    item.id
                })
            })
            .flatten();
        let sort_label: &'static str = if relevance {
            "relevance"
        } else if sort_key == "name" {
            "name"
        } else if sort_key == "file_count" {
            "file_count"
        } else {
            "id"
        };
        Ok(PageEnvelope {
            data,
            meta: PageMeta {
                limit,
                next_cursor,
                has_more,
                sort: Some(sort_label),
                type_counts: Some(type_counts),
            },
        })
    }

    pub fn dataset(&self, id: i64, expose_path: bool) -> Result<DatasetDetail, AppError> {
        if id <= 0 {
            return Err(AppError::BadRequest("dataset id must be positive".into()));
        }
        let conn = self.pool.get()?;
        let mut dataset = conn.query_row(
            "SELECT id,name,dataset_type,species,species_confidence,summary,path,updated_at FROM dataset WHERE id=?1", [id],
            |row| Ok(DatasetDetail { id: row.get(0)?, name: row.get(1)?, dataset_type: row.get(2)?, species: row.get(3)?, species_confidence: row.get(4)?, summary: row.get(5)?, path: expose_path.then(|| row.get(6)).transpose()?, updated_at: row.get(7)?, assets: vec![] })
        ).optional()?.ok_or(AppError::NotFound)?;
        let mut stmt = conn.prepare("SELECT a.id,a.name,a.asset_type,COUNT(af.file_id) FROM asset a LEFT JOIN asset_file af ON af.asset_id=a.id WHERE a.dataset_id=?1 GROUP BY a.id ORDER BY a.id")?;
        dataset.assets = stmt
            .query_map([id], |row| {
                Ok(AssetSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    asset_type: row.get(2)?,
                    file_count: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(dataset)
    }

    pub fn files(
        &self,
        dataset_id: i64,
        query: &FileQuery,
        limit: u32,
        expose_path: bool,
    ) -> Result<PageEnvelope<FileSummary>, AppError> {
        if dataset_id <= 0 || query.asset_id.is_some_and(|id| id <= 0) {
            return Err(AppError::BadRequest("ids must be positive".into()));
        }
        let cursor = decode_cursor(query.cursor)?.unwrap_or(0);
        let conn = self.pool.get()?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dataset WHERE id=?1)",
            [dataset_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(AppError::NotFound);
        }
        let mut stmt = conn.prepare(
            "SELECT f.id,a.id,f.path,f.size,af.role,f.mime_type,f.source_server
             FROM asset a
             CROSS JOIN asset_file af
             CROSS JOIN files f
             WHERE a.dataset_id=?1 AND af.asset_id=a.id AND af.file_id>?2
               AND f.id=af.file_id AND f.deleted=0 AND (?3 IS NULL OR a.id=?3)
             ORDER BY af.file_id LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![dataset_id, cursor, query.asset_id, i64::from(limit + 1)],
            |row| {
                let path: String = row.get(2)?;
                Ok(FileSummary {
                    id: row.get(0)?,
                    asset_id: row.get(1)?,
                    name: file_name(&path),
                    size: row.get::<_, i64>(3)?.max(0) as u64,
                    role: row.get(4)?,
                    mime_type: row.get(5)?,
                    source_server: row.get(6)?,
                    path: expose_path.then_some(path),
                })
            },
        )?;
        let mut data: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
        let has_more = data.len() > limit as usize;
        if has_more {
            data.pop();
        }
        let next_cursor = has_more.then(|| data.last().map(|item| item.id)).flatten();
        Ok(PageEnvelope {
            data,
            meta: PageMeta {
                limit,
                next_cursor,
                has_more,
                sort: None,
                type_counts: None,
            },
        })
    }

    /// Map Tantivy-hit file ids to the datasets that contain those files,
    /// deduplicated per dataset, with the count of matched files per dataset.
    pub fn search_datasets(
        &self,
        file_ids: &[i64],
        expose_path: bool,
    ) -> Result<Vec<DatasetSummary>, AppError> {
        if file_ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.pool.get()?;
        let placeholders = vec!["?"; file_ids.len()].join(",");
        let sql = format!(
            "SELECT d.id,d.name,d.dataset_type,d.species,d.summary,d.path,d.updated_at,
                    COUNT(DISTINCT a.id),COUNT(DISTINCT f.id)
             FROM files f
             JOIN asset_file af ON af.file_id = f.id
             JOIN asset a ON a.id = af.asset_id
             JOIN dataset d ON d.id = a.dataset_id
             WHERE f.id IN ({placeholders})
             GROUP BY d.id
             ORDER BY COUNT(DISTINCT f.id) DESC, d.id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<DatasetSummary> = stmt
            .query_map(rusqlite::params_from_iter(file_ids.iter()), |row| {
                dataset_summary(row, expose_path)
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn facets(&self) -> Result<Facets, AppError> {
        let conn = self.pool.get()?;
        fn values(
            conn: &rusqlite::Connection,
            field: &str,
            limit: i64,
        ) -> rusqlite::Result<Vec<Facet>> {
            let sql = format!(
                "SELECT {field},COUNT(*) FROM dataset WHERE {field} IS NOT NULL AND trim({field})<>'' GROUP BY {field} ORDER BY COUNT(*) DESC,{field} COLLATE NOCASE LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map([limit], |row| {
                Ok(Facet {
                    value: row.get(0)?,
                    count: row.get(1)?,
                })
            })?
            .collect()
        }
        Ok(Facets {
            species: values(&conn, "species", 100)?,
            types: values(&conn, "dataset_type", 100)?,
        })
    }

    pub fn stats(&self) -> Result<Stats, AppError> {
        let conn = self.pool.get()?;
        Ok(Stats {
            datasets_upper_bound: conn.query_row(
                "SELECT COALESCE(MAX(id),0) FROM dataset",
                [],
                |r| r.get(0),
            )?,
            assets_upper_bound: conn.query_row(
                "SELECT COALESCE(MAX(id),0) FROM asset",
                [],
                |r| r.get(0),
            )?,
            files_upper_bound: conn.query_row(
                "SELECT COALESCE(MAX(id),0) FROM files",
                [],
                |r| r.get(0),
            )?,
            linked_files_upper_bound: conn.query_row(
                "SELECT COALESCE(MAX(file_id),0) FROM asset_file",
                [],
                |r| r.get(0),
            )?,
            last_indexed_at: conn
                .query_row(
                    "SELECT updated_at FROM files ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?,
            approximate: true,
        })
    }
}

fn dataset_summary(row: &rusqlite::Row<'_>, expose_path: bool) -> rusqlite::Result<DatasetSummary> {
    Ok(DatasetSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        dataset_type: row.get(2)?,
        species: row.get(3)?,
        summary: row.get(4)?,
        path: expose_path.then(|| row.get(5)).transpose()?,
        updated_at: row.get(6)?,
        asset_count: row.get(7)?,
        file_count: row.get(8)?,
    })
}

fn dataset_type_counts(
    conn: &rusqlite::Connection,
    species: Option<&str>,
    pattern: Option<&str>,
) -> rusqlite::Result<Vec<Facet>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(d.dataset_type,'unknown'),COUNT(*)
         FROM dataset d
         WHERE (?1 IS NULL OR d.species=?1 COLLATE NOCASE)
           AND (?2 IS NULL OR d.name LIKE ?2 ESCAPE char(92) COLLATE NOCASE
                OR d.species LIKE ?2 ESCAPE char(92) COLLATE NOCASE
                OR d.dataset_type LIKE ?2 ESCAPE char(92) COLLATE NOCASE
                OR d.summary LIKE ?2 ESCAPE char(92) COLLATE NOCASE)
         GROUP BY d.dataset_type
         ORDER BY COUNT(*) DESC,d.dataset_type COLLATE NOCASE
         LIMIT 100",
    )?;
    stmt.query_map(params![species, pattern], |row| {
        Ok(Facet {
            value: row.get(0)?,
            count: row.get(1)?,
        })
    })?
    .collect()
}

fn clean_query(value: Option<&str>) -> Result<Option<String>, AppError> {
    let value = value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    if value.as_ref().is_some_and(|v| v.chars().count() > 200) {
        return Err(AppError::BadRequest(
            "q must not exceed 200 characters".into(),
        ));
    }
    Ok(value)
}
fn escape_like(value: &str) -> String {
    format!("%{}%", escape_like_value(value))
}
fn escape_like_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(path)
        .to_owned()
}
fn decode_cursor(cursor: Option<i64>) -> Result<Option<i64>, AppError> {
    if cursor.is_some_and(|v| v < 0) {
        return Err(AppError::BadRequest("cursor must not be negative".into()));
    }
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Settings) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("index.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);
             INSERT INTO dataset VALUES(1,'rice_reference','/data/rice','genome','Oryza_sativa','high','rice reference',10);
             INSERT INTO dataset VALUES(2,'soy_reads','/data/soy','transcriptome','Glycine_max','high','soybean reads',11);
             INSERT INTO dataset VALUES(3,'Oryza','/data/rice/exact','reference','Rice','high','exact name',12);
             INSERT INTO dataset VALUES(4,'Oryza_reads','/data/rice/reads','fastq','Rice','high','prefix name',13);
             INSERT INTO asset VALUES(10,1,'assembly','assembly');
             INSERT INTO files VALUES(100,'/data/rice/genome.fa',123,'text/plain','local',0,12);
             INSERT INTO files VALUES(101,'/data/rice/genes.gff3',456,'text/plain','local',0,13);
             INSERT INTO asset_file VALUES(10,100,'primary');
             INSERT INTO asset_file VALUES(10,101,'annotation');
             INSERT INTO asset VALUES(11,2,'reads','fastq');
             INSERT INTO files VALUES(102,'/data/soy/reads.fastq.gz',789,'application/gzip','local',0,14);
             INSERT INTO asset_file VALUES(11,102,'primary');"
        ).unwrap();
        drop(conn);
        let settings = Settings {
            database: path,
            pool_size: 2,
            ..Settings::default()
        };
        (temp, settings)
    }

    #[test]
    fn searches_dataset_metadata_without_exposing_paths() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let query = DatasetQuery {
            q: Some("Oryza".into()),
            ..DatasetQuery::default()
        };
        let result = db.datasets(&query, 50, false).unwrap();
        let rice = result.data.iter().find(|item| item.id == 1).unwrap();
        assert_eq!(rice.asset_count, 1);
        assert_eq!(rice.file_count, 2);
        assert!(result.data.iter().all(|item| item.path.is_none()));
    }

    #[test]
    fn paginates_files_and_returns_only_basename() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let first = db.files(1, &FileQuery::default(), 1, false).unwrap();
        assert_eq!(first.data[0].name, "genome.fa");
        assert!(first.data[0].path.is_none());
        assert!(first.meta.has_more);
        let second = db
            .files(
                1,
                &FileQuery {
                    cursor: first.meta.next_cursor,
                    ..FileQuery::default()
                },
                1,
                false,
            )
            .unwrap();
        assert_eq!(second.data[0].name, "genes.gff3");
        assert!(!second.meta.has_more);
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let (_temp, mut settings) = fixture();
        settings.supported_schema_versions = vec![5];
        let db = Database::open(&settings).unwrap();
        assert!(matches!(
            db.readiness(&settings.supported_schema_versions),
            Err(AppError::NotReady(_))
        ));
    }

    #[test]
    fn ranks_exact_and_prefix_matches_and_reports_type_counts() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let query = DatasetQuery {
            q: Some("Oryza".into()),
            ..DatasetQuery::default()
        };
        let result = db.datasets(&query, 10, false).unwrap();
        assert_eq!(result.meta.sort, Some("relevance"));
        assert_eq!(
            result.data.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![3, 4, 1]
        );
        let counts = result.meta.type_counts.unwrap();
        assert_eq!(counts.iter().map(|item| item.count).sum::<i64>(), 3);
        assert!(
            counts
                .iter()
                .any(|item| item.value == "fastq" && item.count == 1)
        );
    }

    #[test]
    fn relevance_cursor_has_no_duplicates() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let mut query = DatasetQuery {
            q: Some("Oryza".into()),
            ..DatasetQuery::default()
        };
        let first = db.datasets(&query, 1, false).unwrap();
        query.cursor = first.meta.next_cursor;
        let second = db.datasets(&query, 1, false).unwrap();
        assert_ne!(first.data[0].id, second.data[0].id);
        assert_eq!(second.data[0].id, 4);
    }

    // GUI-T4: sort=name —— 名称不区分大小写升序，id 决胜
    #[test]
    fn sorts_by_name_ascending_without_query() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let query = DatasetQuery {
            sort: Some("name".into()),
            ..DatasetQuery::default()
        };
        let result = db.datasets(&query, 10, false).unwrap();
        assert_eq!(result.meta.sort, Some("name"));
        assert_eq!(
            result.data.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![3, 4, 1, 2]
        );
        // 文件数仍正确聚合
        let rice = result.data.iter().find(|item| item.id == 1).unwrap();
        assert_eq!(rice.file_count, 2);
    }

    // GUI-T4: sort=file_count —— 聚合文件数升序（少→多），id 决胜
    #[test]
    fn sorts_by_file_count_ascending_without_query() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let query = DatasetQuery {
            sort: Some("file_count".into()),
            ..DatasetQuery::default()
        };
        let result = db.datasets(&query, 10, false).unwrap();
        assert_eq!(result.meta.sort, Some("file_count"));
        assert_eq!(
            result.data.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![3, 4, 2, 1]
        );
    }

    // GUI-T4: name 排序用 OFFSET 游标翻页——无重复、覆盖全部
    #[test]
    fn name_sort_paginates_with_offset_cursor() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let mut query = DatasetQuery {
            sort: Some("name".into()),
            ..DatasetQuery::default()
        };
        let first = db.datasets(&query, 2, false).unwrap();
        assert_eq!(
            first.data.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(first.meta.next_cursor, Some(2));
        query.cursor = first.meta.next_cursor;
        let second = db.datasets(&query, 2, false).unwrap();
        assert_eq!(
            second.data.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(!second.meta.has_more);
        assert_eq!(second.meta.next_cursor, None);
    }

    // GUI-T4: 名称排序与关键词过滤组合（q + sort=name 走列排序，非相关度）
    #[test]
    fn name_sort_combines_with_keyword_query() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let query = DatasetQuery {
            q: Some("Oryza".into()),
            sort: Some("name".into()),
            ..DatasetQuery::default()
        };
        let result = db.datasets(&query, 10, false).unwrap();
        assert_eq!(result.meta.sort, Some("name"));
        assert_eq!(
            result.data.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![3, 4, 1]
        );
    }

    #[test]
    fn search_datasets_maps_file_ids_to_datasets() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let rows = db.search_datasets(&[100], false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "rice_reference");
        assert_eq!(rows[0].dataset_type.as_deref(), Some("genome"));
        assert_eq!(rows[0].file_count, 1);
        assert!(rows[0].path.is_none());
    }

    #[test]
    fn search_datasets_deduplicates_files_per_dataset() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let rows = db.search_datasets(&[100, 101], false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 1);
        assert_eq!(rows[0].file_count, 2);
    }

    #[test]
    fn search_datasets_exposes_paths_when_enabled() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        let rows = db.search_datasets(&[100], true).unwrap();
        assert_eq!(rows[0].path.as_deref(), Some("/data/rice"));
    }

    #[test]
    fn search_datasets_returns_empty_for_unknown_ids() {
        let (_temp, settings) = fixture();
        let db = Database::open(&settings).unwrap();
        assert!(db.search_datasets(&[999], false).unwrap().is_empty());
        assert!(db.search_datasets(&[], false).unwrap().is_empty());
    }
}
