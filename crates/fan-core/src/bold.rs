use std::io::Read;
use std::path::Path;
use std::time::Duration;
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
                break;
            }
            in_seq = true;
            seq.push_str(&line);
            seq.push('\n');
            continue;
        }
        if in_seq {
            seq.push_str(&line);
            seq.push('\n');
            if seq.len() >= max_bp {
                break;
            }
        }
    }

    if seq.is_empty() {
        return Err("No sequence found in file".into());
    }

    Ok(seq)
}

/// Identify species using EBI BLAST REST API.
/// Submits a nucleotide BLAST job, polls for completion, and extracts species name.
pub fn identify_species(sequence: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let base_url = "https://www.ebi.ac.uk/Tools/services/rest/ncbiblast";

    // 1. Submit BLAST job
    info!("Submitting EBI BLAST job (sequence length: {} chars)...", sequence.len());
    let submit_response = ureq::post(&format!("{}/run", base_url))
        .timeout(Duration::from_secs(15))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_form(&[
            ("email", "fan-files@example.com"),
            ("program", "blastn"),
            ("database", "ena_sequence"),
            ("sequence", sequence),
            ("stype", "dna"),
        ])
        .map_err(|e| format!("EBI BLAST submit failed: {}", e))?;

    let job_id = submit_response.into_string()
        .map_err(|e| format!("Failed to read job ID: {}", e))?;
    let job_id = job_id.trim();
    info!("EBI BLAST job submitted: {}", job_id);

    // 2. Poll for completion
    let mut attempts = 0;
    let max_attempts = 30;
    loop {
        std::thread::sleep(Duration::from_secs(3));
        attempts += 1;

        let status_response = ureq::get(&format!("{}/status/{}", base_url, job_id))
            .timeout(Duration::from_secs(10))
            .call()
            .map_err(|e| format!("EBI BLAST status check failed: {}", e))?;

        let status = status_response.into_string()?;
        let status = status.trim();

        if status == "FINISHED" {
            info!("EBI BLAST job finished after {} attempts", attempts);
            break;
        }
        if status == "ERROR" || status == "FAILURE" {
            return Err(format!("EBI BLAST job failed: {}", status).into());
        }
        if attempts >= max_attempts {
            return Err("EBI BLAST job timed out".into());
        }

        if attempts % 5 == 0 {
            info!("EBI BLAST job status: {} (attempt {}/{})", status, attempts, max_attempts);
        }
    }

    // 3. Get results
    let result_response = ureq::get(&format!("{}/result/{}/out", base_url, job_id))
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| format!("EBI BLAST result fetch failed: {}", e))?;

    let result_text = result_response.into_string()?;

    // 4. Parse species from BLAST results
    let species = parse_species_from_blast_output(&result_text);
    Ok(species)
}

/// Parse the best species hit from BLAST tabular output.
/// Looks for the first hit's description line and extracts species from OS=... pattern
/// or from the hit definition line that typically contains Latin binomial names.
fn parse_species_from_blast_output(text: &str) -> Option<String> {
    let mut best_species: Option<String> = None;
    let mut best_score = 0.0;

    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // BLAST tabular format: qseqid sseqid pident length ... evalue bitscore
        // Hit descriptions contain species info
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        // Parse bitscore (last field or second to last)
        let bitscore: f64 = fields.last()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        if bitscore > best_score {
            best_score = bitscore;

            // Try to extract species from the hit description
            // EBI BLAST description format often includes "OS=Species Name"
            let desc = if fields.len() > 12 { fields.get(12) } else { None };

            let species = if let Some(desc) = desc {
                // Try OS=... pattern (EMBL format)
                if let Some(os_pos) = desc.find("OS=") {
                    let after_os = &desc[os_pos + 3..];
                    after_os.split(|c| c == '=' || c == ';' || c == '\n')
                        .next()
                        .map(|s| s.trim().to_string())
                } else {
                    // Try Latin binomial from hit ID
                    // Often format like "gb|XXX|Genus_species"
                    let hit_id = fields.get(1).unwrap_or(&"");
                    let parts: Vec<&str> = hit_id.split('|').collect();
                    let last_part = parts.last().unwrap_or(&"");
                    // Convert underscores to spaces for genus_species → genus species
                    let clean = last_part.replace('_', " ");
                    if clean.split_whitespace().count() >= 2 {
                        Some(clean)
                    } else {
                        None
                    }
                }
            } else {
                None
            };

            if let Some(sp) = species {
                best_species = Some(sp);
            }
        }
    }

    info!("EBI BLAST top species: {:?} (bitscore: {})", best_species, best_score);
    best_species
}
