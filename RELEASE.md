# fan-files v0.2.1 — Post Phase C Performance Fix & Stability Improvements

> Bugfix release: eliminates ~18h hang on production-scale data, resolves FOREIGN KEY errors, enriches search index.

## Fixes

| Fix | Description | Impact |
|-----|-------------|--------|
| Post Phase C O(N×M) → O(M log N) | Sort-merge replaces double loop for auxiliary file linking | Eliminates ~18h hang on 6.5M files |
| FOREIGN KEY via UPSERT | `INSERT OR REPLACE` → `ON CONFLICT DO UPDATE ... RETURNING id` | Zero FOREIGN KEY errors |
| LLM API 120s timeout | `ureq::AgentBuilder::timeout` prevents worker hang | Gateway stall no longer freezes Phase C |
| Tantivy enrichment | `dataset_type` + `species` written to search index after Phase C | `fan-files search Glycine_max` now works |
| Binary search boundary fix | `partition_point` condition corrected for prefix matching | All files correctly assigned to datasets |
| rebuild-index command | New `fan-files rebuild-index` for manual index rebuild | One-time fix for pre-v0.2.1 databases |

## Validation

- 3 new unit tests: UPSERT id preservation, prefix matching boundaries, empty input handling
- Production-tested on 58 server (6.5M files, 8,160 datasets)
- Post Phase C: 3.35s (was ~18h)

## Assets

- `fan-files-linux-x86_64.tar.gz` — Linux (glibc, x86_64)
- `fan-files-macos-arm64.tar.gz` — macOS (Apple Silicon)

## Installation

```bash
tar xzf fan-files-linux-x86_64.tar.gz
sudo mv fan-files /usr/local/bin/
fan-files --version
```
