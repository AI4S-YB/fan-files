# fan-files — Data Sharing Sidecar

## New: fan-files-share

A read-only HTTP API sidecar that exposes fan-files indexed Dataset metadata to downstream services (e.g., TeamDesk).

- Independent crate, no dependency on fan-core
- Stack: axum + tokio + rusqlite + r2d2 + tower-http
- Strict SQLite read-only boundary (SQLITE_OPEN_READ_ONLY + query_only)
- Path hiding by default (no absolute paths exposed)
- Cursor pagination with relevance-ranked search
- healthz / readyz separation with schema version validation

### API Endpoints

| Endpoint | Purpose |
|----------|---------|
| `/healthz` | Liveness |
| `/readyz` | Readiness (schema version + table check) |
| `/api/v1/datasets` | List, search, filter, paginate |
| `/api/v1/datasets/:id` | Detail with assets |
| `/api/v1/datasets/:id/files` | File pagination |
| `/api/v1/facets` | Species/type facets |
| `/api/v1/stats` | Approximate stats |

### Validation

- 7/7 unit tests
- 28,655 real-database requests, 100% HTTP 200
- 10/50 concurrency: P95 88ms / 310ms
- syscall audit: zero forbidden writes during concurrent fan-files writes

## Assets

- `fan-files-linux-x86_64.tar.gz`
- `fan-files-macos-arm64.tar.gz`
