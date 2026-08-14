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
}
