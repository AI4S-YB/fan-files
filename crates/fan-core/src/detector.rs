use fan_plugin_sdk::FormatInfo;

/// Built-in format detection (Rust, no WASM needed).
/// Detects common bioinformatics file formats by magic bytes and extension.
pub struct BuiltinDetector;

impl BuiltinDetector {
    pub fn detect(path: &str, magic: &[u8]) -> Option<FormatInfo> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let file_type = detect_by_ext_and_magic(&ext, magic);
        let mime = mime_for_type(&file_type);

        Some(FormatInfo {
            file_type: file_type.to_string(),
            mime: Some(mime.to_string()),
        })
    }
}

fn detect_by_ext_and_magic(ext: &str, magic: &[u8]) -> String {
    // Magic byte checks first
    if magic.len() >= 4 {
        if magic[..4] == *b"BAM\x01" {
            return "BAM".into();
        }
        if magic.starts_with(b"CRAM") {
            return "CRAM".into();
        }
        if magic.starts_with(b"@HD\t") {
            return "SAM".into();
        }
        if magic.starts_with(b"##fileformat=VCF") {
            return "VCF".into();
        }
        if magic.starts_with(b"\x89HDF") {
            return "HDF5".into();
        }
        if magic.starts_with(b"HDF\r") {
            return "HDF5".into();
        }
        if magic.starts_with(b"\x1f\x8b") {
            return format!("gzip({})", ext_to_type(ext));
        }
    }

    // Check for FASTA (>header) or FASTQ (@header)
    if !magic.is_empty() {
        if magic[0] == b'>' {
            return "FASTA".into();
        }
        if magic[0] == b'@' {
            // Could be FASTQ or SAM — SAM header starts with @HD, @SQ, @RG, @PG
            let header = std::str::from_utf8(magic.get(..20).unwrap_or(magic)).unwrap_or("");
            if header.starts_with("@HD")
                || header.starts_with("@SQ")
                || header.starts_with("@RG")
                || header.starts_with("@PG")
            {
                return "SAM".into();
            }
            return "FASTQ".into();
        }
    }

    // Extension-based fallback
    ext_to_type(ext)
}

fn ext_to_type(ext: &str) -> String {
    match ext {
        "fastq" | "fq" => "FASTQ",
        "fasta" | "fa" | "fna" | "faa" | "ffn" => "FASTA",
        "bam" => "BAM",
        "sam" => "SAM",
        "cram" => "CRAM",
        "vcf" => "VCF",
        "bcf" => "BCF",
        "gff" | "gtf" | "gff3" => "GFF",
        "bed" => "BED",
        "bw" | "bigwig" => "bigWig",
        "bg" | "bedgraph" => "bedGraph",
        "h5" | "hdf5" | "h5ad" => "HDF5",
        "csv" => "CSV",
        "tsv" | "tab" => "TSV",
        "txt" => "text",
        "json" => "JSON",
        "gz" => "gzip",
        "bz2" => "bzip2",
        "mtx" => "MatrixMarket",
        "rds" => "RData",
        _ => "unknown",
    }
    .to_string()
}

fn mime_for_type(file_type: &str) -> &'static str {
    match file_type {
        "FASTQ" | "FASTA" | "SAM" | "VCF" | "GFF" | "BED" | "CSV" | "TSV" => "text/plain",
        "BAM" | "CRAM" | "BCF" | "HDF5" | "bigWig" | "bedGraph" => {
            "application/octet-stream"
        }
        "gzip" => "application/gzip",
        _ if file_type.starts_with("gzip") => "application/gzip",
        "JSON" => "application/json",
        _ => "application/octet-stream",
    }
}
