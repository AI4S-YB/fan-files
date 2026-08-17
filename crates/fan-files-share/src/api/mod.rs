use crate::{
    error::AppError,
    models::*,
    state::{AppState, Cache},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use fan_core::index::tantivy::TantivyIndex;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/api/v1/datasets", get(datasets))
        .route("/api/v1/datasets/{id}", get(dataset))
        .route("/api/v1/datasets/{id}/files", get(files))
        .route("/api/v1/facets", get(facets))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/search", get(search))
        .with_state(state)
}

async fn health() -> Json<Envelope<Health>> {
    Json(Envelope {
        data: Health { status: "ok" },
    })
}
async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<Envelope<Readiness>>, AppError> {
    let db = state.db.clone();
    let supported = state.settings.supported_schema_versions.clone();
    let version = blocking(move || db.readiness(&supported)).await?;
    Ok(Json(Envelope {
        data: Readiness {
            status: "ready",
            database: "ok",
            schema_version: version,
        },
    }))
}
async fn datasets(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DatasetQuery>,
) -> Result<Json<PageEnvelope<DatasetSummary>>, AppError> {
    validate_dataset_order(&query)?;
    let limit = page_limit(query.limit, state.settings.max_page_size)?;
    let db = state.db.clone();
    let expose = state.settings.expose_absolute_paths;
    Ok(Json(
        blocking(move || db.datasets(&query, limit, expose)).await?,
    ))
}
fn validate_dataset_order(query: &DatasetQuery) -> Result<(), AppError> {
    if query
        .sort
        .as_deref()
        .is_some_and(|value| value != "id" && value != "relevance")
    {
        return Err(AppError::BadRequest("sort must be id or relevance".into()));
    }
    if query.sort.as_deref() == Some("relevance")
        && query
            .q
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(AppError::BadRequest("sort=relevance requires q".into()));
    }
    if query.order.as_deref().is_some_and(|value| value != "asc") {
        return Err(AppError::BadRequest(
            "v1 cursor pagination supports order=asc only".into(),
        ));
    }
    Ok(())
}
async fn dataset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Envelope<DatasetDetail>>, AppError> {
    let db = state.db.clone();
    let expose = state.settings.expose_absolute_paths;
    Ok(Json(Envelope {
        data: blocking(move || db.dataset(id, expose)).await?,
    }))
}
async fn files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<FileQuery>,
) -> Result<Json<PageEnvelope<FileSummary>>, AppError> {
    let limit = page_limit(query.limit, state.settings.max_page_size)?;
    let db = state.db.clone();
    let expose = state.settings.expose_absolute_paths;
    Ok(Json(
        blocking(move || db.files(id, &query, limit, expose)).await?,
    ))
}
async fn facets(State(state): State<Arc<AppState>>) -> Result<Json<Envelope<Facets>>, AppError> {
    let ttl = Duration::from_secs(state.settings.stats_cache_seconds);
    if let Some(value) = cached(&state.facets, ttl) {
        return Ok(Json(Envelope { data: value }));
    }
    let db = state.db.clone();
    let value = blocking(move || db.facets()).await?;
    *state.facets.lock().map_err(|_| AppError::Internal)? = Some(Cache {
        loaded: Instant::now(),
        value: value.clone(),
    });
    Ok(Json(Envelope { data: value }))
}
async fn stats(State(state): State<Arc<AppState>>) -> Result<Json<Envelope<Stats>>, AppError> {
    let ttl = Duration::from_secs(state.settings.stats_cache_seconds);
    if let Some(value) = cached(&state.stats, ttl) {
        return Ok(Json(Envelope { data: value }));
    }
    let db = state.db.clone();
    let value = blocking(move || db.stats()).await?;
    *state.stats.lock().map_err(|_| AppError::Internal)? = Some(Cache {
        loaded: Instant::now(),
        value: value.clone(),
    });
    Ok(Json(Envelope { data: value }))
}
async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Envelope<Vec<DatasetSummary>>>, AppError> {
    let q = query.q.trim().to_string();
    if q.is_empty() {
        return Err(AppError::BadRequest("q is required".into()));
    }
    let expose = state.settings.expose_absolute_paths;
    let datasets = blocking(move || run_search(&state, &q, expose)).await?;
    Ok(Json(Envelope { data: datasets }))
}

/// Run a full-text search over the Tantivy index and map the hit file ids
/// back to their datasets. Returns an empty list when the index does not
/// exist (e.g. the desktop CLI has not built one yet).
fn run_search(state: &AppState, q: &str, expose_path: bool) -> Result<Vec<DatasetSummary>, AppError> {
    let file_ids = tantivy_file_ids(state, q)?;
    state.db.search_datasets(&file_ids, expose_path)
}

/// Lock the shared index, lazily opening it on first use so that a share
/// started before the CLI built the index picks it up without a restart.
/// Open failures are logged and retried on the next call; the open is
/// guarded by the existence of the `<data_dir>/tantivy` directory.
fn tantivy_file_ids(state: &AppState, q: &str) -> Result<Vec<i64>, AppError> {
    let mut index = state.tantivy.lock().map_err(|_| AppError::Internal)?;
    if index.is_none() {
        let data_dir = state
            .settings
            .database
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        if data_dir.join("tantivy").exists() {
            match TantivyIndex::open(&data_dir, true) {
                Ok(opened) => {
                    index.replace(opened);
                }
                Err(error) => tracing::warn!(%error, "tantivy open failed"),
            }
        }
    }
    let Some(index) = index.as_ref() else {
        return Ok(vec![]);
    };
    let hits = index.search(q, 200).map_err(|error| {
        tracing::error!(%error, "tantivy search failed");
        AppError::Internal
    })?;
    Ok(hits.into_iter().map(|(id, _)| id).collect())
}

fn cached<T: Clone>(cache: &std::sync::Mutex<Option<Cache<T>>>, ttl: Duration) -> Option<T> {
    cache
        .lock()
        .ok()?
        .as_ref()
        .filter(|item| item.loaded.elapsed() < ttl)
        .map(|item| item.value.clone())
}
fn page_limit(value: Option<u32>, max: u32) -> Result<u32, AppError> {
    let value = value.unwrap_or(50);
    if value == 0 || value > max {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {max}"
        )));
    }
    Ok(value)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        tracing::error!(%error, "database worker failed");
        AppError::Internal
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use fan_core::index::tantivy::TantivyIndex;
    use serde_json::Value;
    use tower::ServiceExt;

    #[test]
    fn accepts_relevance_only_with_query() {
        let valid = DatasetQuery {
            q: Some("Oryza".into()),
            sort: Some("relevance".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&valid).is_ok());

        let missing_query = DatasetQuery {
            sort: Some("relevance".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&missing_query).is_err());
    }

    #[test]
    fn rejects_unknown_sort_and_oversized_page() {
        let invalid = DatasetQuery {
            sort: Some("name".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&invalid).is_err());
        assert!(page_limit(Some(201), 200).is_err());
        assert_eq!(page_limit(None, 200).unwrap(), 50);
    }

    fn api_fixture(db_path: &std::path::Path) -> Settings {
        Settings {
            database: db_path.to_path_buf(),
            pool_size: 2,
            ..Settings::default()
        }
    }

    fn create_db(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn search_endpoint_returns_empty_without_tantivy_index() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search?q=genome")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn search_endpoint_returns_datasets() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);
             INSERT INTO dataset VALUES(1,'Oryza_sativa_v1','/data/orders/Poales/Poaceae/Oryza_sativa/v1','genome','Oryza sativa',NULL,'rice reference genome',0);
             INSERT INTO asset VALUES(1,1,'assembly','assembly');
             INSERT INTO files VALUES(10,'/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa',123,'text/plain','local',0,0);
             INSERT INTO asset_file VALUES(1,10,'primary');",
        )
        .unwrap();
        drop(conn);
        // Build a tiny tantivy index next to the database, as the CLI would.
        let index = TantivyIndex::open(temp.path(), false).unwrap();
        index
            .index_file(
                10,
                std::path::Path::new("/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa"),
                "Oryza sativa reference genome assembly",
                &[],
            )
            .unwrap();
        index.commit().unwrap();
        drop(index);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search?q=genome")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["name"], "Oryza_sativa_v1");
        assert_eq!(data[0]["type"], "genome");
        assert_eq!(data[0]["file_count"], 1);
    }

    #[tokio::test]
    async fn search_endpoint_requires_nonempty_q() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));
        for uri in ["/api/v1/search", "/api/v1/search?q="] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "uri: {uri}");
        }
    }
}
