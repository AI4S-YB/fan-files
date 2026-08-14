# fan-files-share

`fan-files-share` is an optional, read-only sidecar for fan-files. It exposes Dataset metadata to TeamDesk without modifying or embedding an HTTP server in the main fan-files process.

## v1 scope

- SQLite Dataset metadata search (`name`, `species`, `dataset_type`, `summary`)
- species/type filtering and Dataset cursor pagination
- Dataset and Asset details
- paginated Files
- cached facets and statistics
- separate liveness and readiness checks

Tantivy file full-text search is intentionally deferred to v2.

## API

```text
GET /healthz
GET /readyz
GET /api/v1/datasets?q=&species=&type=&cursor=&limit=&sort=relevance|id&order=asc
GET /api/v1/datasets/:id
GET /api/v1/datasets/:id/files?asset_id=&cursor=&limit=
GET /api/v1/facets
GET /api/v1/stats
```

Paths are hidden by default. File responses expose only the basename and stable IDs.
Keyword queries default to lightweight relevance ordering and return per-type hit counts in
`meta.type_counts`. Pass `sort=id` for stable ID ordering. Dataset and File pagination both
continue with the opaque value returned in `meta.next_cursor`.

## Run

```bash
cargo run --release -- --config config/example.toml
```

or override the essential settings:

```bash
fan-files-share --database ~/.fan-files/data/index.db --bind 127.0.0.1:8932
```

The database must already exist. The service opens SQLite with `SQLITE_OPEN_READ_ONLY`, enables `PRAGMA query_only=ON`, and never runs migrations or lock cleanup.

## Production validation

Before enabling TeamDesk traffic, validate on the 58 server using the real database:

- `/readyz` reports schema version 4;
- fan-files writes and sidecar reads coexist through SQLite WAL;
- Dataset list/detail p95 is below 200 ms;
- Dataset metadata search and File pages p95 are below 300 ms;
- cached facets/stats p95 is below 100 ms;
- concurrency 10 and 50 do not cause unbounded waits;
- large Datasets remain usable with File pages capped at 200.

The TeamDesk search UI must label v1 search as **Dataset metadata search**, not file full-text search.

The validated read-only pool size is 4. On the 58 server, Dataset queries remained available
while fan-files updated the same WAL database under concurrency 10 and 50. The sidecar made no
write, truncate, rename, or unlink system calls against the SQLite database or WAL during the run.

`/api/v1/stats` intentionally returns inexpensive upper bounds with `approximate=true`. Exact
counts over millions of File rows are outside the synchronous v1 request path.
