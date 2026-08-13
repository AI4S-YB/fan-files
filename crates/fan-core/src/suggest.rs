use crate::index::sqlite::SqliteStore;
use crate::types::DatasetEntry;
use fan_plugin_sdk::{DataSource, SearchResult};
use std::collections::HashSet;
use tracing::info;

/// Complementary experiment/data types. Dataset inference sometimes emits
/// broad names (for example `transcriptome`) and sometimes assay names, so the
/// matrix intentionally covers both forms.
static COMPLEMENTARY_TYPES: &[(&str, &[&str])] = &[
    ("RNA-seq", &["ChIP-seq", "ATAC-seq", "WGBS"]),
    ("transcriptome", &["epigenome", "genome", "proteome"]),
    ("WGS", &["WGBS", "RNA-seq", "ChIP-seq"]),
    (
        "genome",
        &["transcriptome", "genome_annotation", "epigenome"],
    ),
    (
        "genome_annotation",
        &["transcriptome", "proteome", "functional"],
    ),
    ("scRNA-seq", &["scATAC-seq", "CITE-seq"]),
    ("ChIP-seq", &["RNA-seq", "ATAC-seq"]),
    ("ATAC-seq", &["RNA-seq", "ChIP-seq"]),
    ("WGBS", &["RNA-seq", "WGS"]),
];

pub struct SuggestEngine;

impl SuggestEngine {
    pub fn suggest(
        sqlite: &SqliteStore,
        project_dir: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let Some(current) = sqlite.find_dataset_for_path(project_dir)? else {
            info!("No inferred dataset found for path: {}", project_dir);
            return Ok(Vec::new());
        };
        let Some(species) = current.species.as_deref().filter(|value| !value.is_empty()) else {
            info!("Dataset {} has no species metadata", current.path);
            return Ok(Vec::new());
        };

        // Fetch a bounded candidate pool. A larger pool than the output limit
        // lets complementary types rank ahead of generic same-species matches.
        let candidate_limit = limit.saturating_mul(20).max(100);
        let candidates = sqlite.datasets_by_species(species, current.id, candidate_limit)?;
        let wanted = complementary_types(current.dataset_type.as_deref());

        let mut scored: Vec<SearchResult> = candidates
            .into_iter()
            .filter_map(|candidate| score_dataset(&current, candidate, &wanted))
            .collect();
        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.path.cmp(&right.path))
        });
        scored.truncate(limit);
        info!(
            "Returning {} dataset suggestions for {}",
            scored.len(),
            current.path
        );
        Ok(scored)
    }
}

fn complementary_types(dataset_type: Option<&str>) -> HashSet<String> {
    let Some(dataset_type) = dataset_type else {
        return HashSet::new();
    };
    COMPLEMENTARY_TYPES
        .iter()
        .filter(|(source, _)| source.eq_ignore_ascii_case(dataset_type))
        .flat_map(|(_, targets)| targets.iter().map(|target| target.to_ascii_lowercase()))
        .collect()
}

fn score_dataset(
    current: &DatasetEntry,
    candidate: DatasetEntry,
    wanted: &HashSet<String>,
) -> Option<SearchResult> {
    let candidate_type = candidate.dataset_type.as_deref().unwrap_or("");
    let same_type = current
        .dataset_type
        .as_deref()
        .is_some_and(|source| source.eq_ignore_ascii_case(candidate_type));
    let complementary = wanted.contains(&candidate_type.to_ascii_lowercase());

    let mut score = 0.5; // same species: enforced by the SQL query
    let mut reasons = vec!["same species".to_string()];
    if complementary {
        score += 0.35;
        reasons.push("complementary dataset type".to_string());
    } else if !same_type && !candidate_type.is_empty() {
        score += 0.15;
        reasons.push("different dataset type".to_string());
    }
    if candidate.species_confidence.as_deref() == Some("high") {
        score += 0.05;
        reasons.push("high-confidence species".to_string());
    }

    Some(SearchResult {
        path: candidate.path,
        score,
        file_type: candidate.dataset_type.clone(),
        assay_type: candidate.dataset_type,
        species: candidate.species,
        tags: Vec::new(),
        summary: reasons.join(", "),
        source: DataSource::Local,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset(id: i64, path: &str, dataset_type: &str) -> DatasetEntry {
        DatasetEntry {
            id,
            name: format!("dataset-{id}"),
            path: path.to_string(),
            dataset_type: Some(dataset_type.to_string()),
            species: Some("Oryza sativa".to_string()),
            species_confidence: Some("high".to_string()),
            species_source: None,
            summary: None,
            indexed_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn complementary_dataset_ranks_above_same_type() {
        let current = dataset(1, "/data/rice/rna", "RNA-seq");
        let wanted = complementary_types(current.dataset_type.as_deref());
        let complementary =
            score_dataset(&current, dataset(2, "/data/rice/atac", "ATAC-seq"), &wanted).unwrap();
        let same =
            score_dataset(&current, dataset(3, "/data/rice/rna-2", "RNA-seq"), &wanted).unwrap();
        assert!(complementary.score > same.score);
    }
}
