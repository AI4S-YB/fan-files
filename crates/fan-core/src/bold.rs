use std::io::Read;
use std::path::Path;
use tracing::info;

/// 在项目目录中自动选择最适合的序列文件用于物种鉴定
pub fn find_blast_file(project_dirs: &[String]) -> Option<String> {
    let mut candidates: Vec<(String, u64)> = Vec::new();
    let seq_exts = [
        "fa.gz", "fasta.gz", "fa", "fasta", "fna", "fastq.gz", "fq.gz", "fastq",
    ];

    for dir in project_dirs {
        let path = Path::new(dir);
        if !path.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let fpath = entry.path();
                let name = fpath
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                let name_lower = name.to_lowercase();

                // Skip annotation/config files
                if name_lower.contains("func_anno")
                    || name_lower.contains("go_")
                    || name_lower.contains("kegg")
                    || name_lower.ends_with(".json")
                    || name_lower.ends_with(".xml")
                    || name_lower.ends_with(".txt")
                {
                    continue;
                }

                if seq_exts.iter().any(|ext| name_lower.ends_with(ext)) {
                    if let Ok(meta) = std::fs::metadata(&fpath) {
                        candidates.push((fpath.to_string_lossy().to_string(), meta.len()));
                    }
                }
            }
        }
    }

    // Also look one level down in subdirectories
    if candidates.is_empty() {
        for dir in project_dirs {
            if let Ok(entries) = std::fs::read_dir(Path::new(dir)) {
                for entry in entries.flatten() {
                    let subdir = entry.path();
                    if subdir.is_dir() {
                        if let Ok(sub_entries) = std::fs::read_dir(&subdir) {
                            for sub_entry in sub_entries.flatten() {
                                let fpath = sub_entry.path();
                                let name = fpath
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("");
                                let name_lower = name.to_lowercase();
                                if seq_exts.iter().any(|ext| name_lower.ends_with(ext)) {
                                    if let Ok(meta) = std::fs::metadata(&fpath) {
                                        candidates
                                            .push((fpath.to_string_lossy().to_string(), meta.len()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort: FASTA-like first (non-fastq), then by size desc
    candidates.sort_by(|a, b| {
        let a_is_fastq = a.0.to_lowercase().contains("fastq");
        let b_is_fastq = b.0.to_lowercase().contains("fastq");
        a_is_fastq.cmp(&b_is_fastq).then(b.1.cmp(&a.1))
    });

    candidates.first().map(|(p, s)| {
        info!("Selected BLAST file: {} ({} bytes)", p, s);
        p.clone()
    })
}

/// Extract first max_bp base pairs of sequence from a FASTA file (supports .gz)
pub fn extract_sequence(
    file_path: &str,
    max_bp: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = Path::new(file_path);
    let file = std::fs::File::open(path)?;
    let reader: Box<dyn Read> = if file_path.ends_with(".gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    use std::io::BufRead;
    let mut seq = String::new();
    let mut in_seq = false;
    for line in std::io::BufReader::new(reader).lines() {
        let line = line?;
        if line.starts_with('>') {
            if in_seq {
                break; // second header, stop
            }
            in_seq = true;
            continue;
        }
        if in_seq {
            seq.push_str(&line);
            if seq.len() >= max_bp {
                break;
            }
        }
    }

    if seq.is_empty() {
        return Err("No sequence found in file".into());
    }

    Ok(seq[..seq.len().min(max_bp)].to_uppercase())
}

/// Call BOLD API to identify species from a DNA sequence
pub fn identify_species(sequence: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let url = "https://v4.boldsystems.org/index.php/Ids_OpenApi";
    info!("Calling BOLD API (sequence length: {}bp)...", sequence.len());

    let response = ureq::post(url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .timeout(std::time::Duration::from_secs(10))
        .send_form(&[
            ("sequence", sequence),
            ("db", "COX1_SPECIES_PUBLIC,COX1,COX1_SPECIES"),
            ("format", "json"),
        ])
        .map_err(|e| format!("BOLD API call failed: {}", e))?;

    let json: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("BOLD response parse error: {}", e))?;

    // Parse top match
    if let Some(matches) = json["top_matches"].as_array() {
        if let Some(top) = matches.first() {
            let species = top["taxonomicidentification"]
                .as_str()
                .or_else(|| top["species_name"].as_str())
                .map(|s| s.to_string());
            let similarity = top["similarity"].as_f64().unwrap_or(0.0);
            if similarity > 95.0 {
                return Ok(species);
            }
        }
    }

    Ok(None)
}
