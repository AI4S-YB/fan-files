use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_plugin_sdk::{DataSource, SearchResult};
use std::collections::HashMap;

pub fn run(config: &Config, query: &str, json: bool) {
    let index = match IndexEngine::open(config, true) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    // 1. Tantivy full-text search
    let tantivy_results = index.tantivy.search(query, 50).unwrap_or_default();

    // If Tantivy returns nothing, fall back to SQLite LIKE search
    let mut file_ids: Vec<(i64, f32)> = if tantivy_results.is_empty() {
        index.sqlite.search_by_metadata(query, 50).unwrap_or_default()
            .into_iter().map(|(id, score)| (id, score as f32)).collect()
    } else {
        tantivy_results
    };

    let max_tantivy_score = file_ids.iter().map(|(_, s)| *s).fold(0.0f32, f32::max).max(1.0);

    // 2. Semantic embedding search (if model available)
    let query_embedding = index.embedding.embed(query).ok();
    let embedding_scores: HashMap<i64, f64> = if let Some(ref qvec) = query_embedding {
        let stored = index.sqlite.load_embeddings().unwrap_or_default();
        let mut scores = HashMap::new();
        for (file_id, vec) in &stored {
            if vec.len() == qvec.len() {
                scores.insert(*file_id, cosine_similarity(qvec, vec));
            }
        }
        scores
    } else {
        HashMap::new()
    };
    let has_embeddings = !embedding_scores.is_empty();

    // 3. Merge and score: Tantivy/SQLite (0.6) + Embedding (0.4)
    let mut merged: HashMap<i64, (f64, f32)> = HashMap::new();
    for (file_id, tantivy_score) in &file_ids {
        let norm_tantivy = *tantivy_score as f64 / max_tantivy_score as f64;
        let emb_score = embedding_scores.get(file_id).copied().unwrap_or(0.0);
        let combined = if has_embeddings {
            norm_tantivy * 0.6 + emb_score * 0.4
        } else {
            norm_tantivy
        };
        merged.insert(*file_id, (combined, *tantivy_score));
    }
    // Also include files found only by embedding
    for (file_id, emb_score) in &embedding_scores {
        if !merged.contains_key(file_id) && *emb_score > 0.3 {
            merged.insert(*file_id, (*emb_score, 0.0));
        }
    }

    // Sort by combined score
    let mut sorted: Vec<(i64, (f64, f32))> = merged.into_iter().collect();
    sorted.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap_or(std::cmp::Ordering::Equal));
    sorted.truncate(20);

    // 4. Build results
    let mut results: Vec<SearchResult> = Vec::new();
    for (file_id, (combined_score, _)) in &sorted {
        if let Ok(Some(entry)) = index.sqlite.get_by_id(*file_id) {
            let mut r = SearchResult {
                path: entry.path.to_string_lossy().to_string(),
                score: *combined_score,
                file_type: entry.format_info.as_ref().map(|f| f.file_type.clone()),
                assay_type: entry
                    .bio_metadata
                    .as_ref()
                    .and_then(|m| m.assay_type.clone()),
                species: entry
                    .bio_metadata
                    .as_ref()
                    .and_then(|m| m.species.clone()),
                tags: entry
                    .bio_metadata
                    .as_ref()
                    .map(|m| m.tags.clone())
                    .unwrap_or_default(),
                summary: entry
                    .bio_metadata
                    .as_ref()
                    .map(|m| {
                        let parts: Vec<String> = vec![
                            m.assay_type.clone(),
                            m.species.clone(),
                            m.tissue.clone(),
                            m.project.clone(),
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        parts.join(", ")
                    })
                    .unwrap_or_default(),
                source: DataSource::Local,
            };
            // Enrich with project info
            if let Some(proj_name) = get_project_for_path(&index.sqlite, &r.path) {
                r.summary = format!("[project: {}] {}", proj_name, r.summary);
            }
            results.push(r);
        }
    }


    // Check metadata coverage
    let total = index.sqlite.status().unwrap().indexed_files;
    let with_meta = index.sqlite.count_with_bio_metadata().unwrap_or(0);
    let coverage_pct = if total > 0 { (with_meta as f64 / total as f64) * 100.0 } else { 0.0 };

    if coverage_pct < 50.0 && total > 10 && !json {
        eprintln!("⚠  Metadata coverage is low ({:.0}%). Run 'fan-files infer' for better results.", coverage_pct);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        for r in &results {
            println!(
                "{:.3}  {}  {:?}  {:?}  {}",
                r.score, r.path, r.assay_type, r.species, r.summary
            );
        }
        if results.is_empty() {
            println!("No results found for: {}", query);
        }
    }
}

fn get_project_for_path(sqlite: &fan_core::index::sqlite::SqliteStore, file_path: &str) -> Option<String> {
    let conn = sqlite.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT p.name FROM project p
         JOIN project_file pf ON p.id = pf.project_id
         JOIN files f ON f.id = pf.file_id
         WHERE f.path = ?1 LIMIT 1"
    ).ok()?;
    stmt.query_row(rusqlite::params![file_path], |row| row.get(0)).ok()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let na: f64 = a.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}
