/// Magic byte signatures for common bioinformatics file formats
static MAGIC_SIGS: &[(&[u8], &str)] = &[
    (b"\x1f\x8b", "gzip"),
    (b"BZh", "bzip2"),
    (b"PK\x03\x04", "zip"),
    (b"\x89PNG\r\n\x1a\n", "png"),
    (b"CRAM", "cram"),
    (b"BAM\x01", "bam"),
    (b"@HD\t", "sam"),
    (b"##fileformat=VCF", "vcf"),
    (b"HDF\r\n\x1a\n", "hdf5"),
    (b"\x89HDF\r\n\x1a\n", "hdf5"),
    (b"\x93NUMPY", "npy"),
    (b"RIFF", "riff"),
];

/// Known bioinformatics extensions
fn classify_by_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "fastq" | "fq" => Some("fastq"),
        "fasta" | "fa" | "fna" | "faa" | "ffn" | "frn" => Some("fasta"),
        "bam" => Some("bam"),
        "sam" => Some("sam"),
        "cram" => Some("cram"),
        "vcf" => Some("vcf"),
        "bcf" => Some("bcf"),
        "gff" | "gtf" | "gff3" => Some("gff"),
        "bed" => Some("bed"),
        "bw" | "bigwig" => Some("bigwig"),
        "bg" | "bedgraph" => Some("bedgraph"),
        "h5" | "hdf5" | "h5ad" | "loom" => Some("hdf5"),
        "csv" => Some("csv"),
        "tsv" | "tab" => Some("tsv"),
        "txt" => Some("text"),
        "gz" => Some("gzip"),
        "bz2" => Some("bzip2"),
        "zip" => Some("zip"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "rds" | "rda" => Some("r-data"),
        "mtx" => Some("matrix-market"),
        _ => None,
    }
}

/// Export: check if this detector can handle the file
#[unsafe(no_mangle)]
pub unsafe extern "C" fn can_handle(_path_ptr: *const u8, _path_len: usize, magic_ptr: *const u8, magic_len: usize) -> i32 {
    let magic = unsafe { std::slice::from_raw_parts(magic_ptr, magic_len.min(16)) };

    // Check magic signatures
    for (sig, _name) in MAGIC_SIGS {
        if magic.starts_with(sig) {
            return 1;
        }
    }

    // Special checks
    if magic.starts_with(b">") || magic.starts_with(b"@") {
        return 1;
    }

    0
}

/// Export: detect format and return JSON
#[unsafe(no_mangle)]
pub unsafe extern "C" fn detect(path_ptr: *const u8, path_len: usize, magic_ptr: *const u8, magic_len: usize) -> i32 {
    let path = unsafe { std::str::from_utf8(std::slice::from_raw_parts(path_ptr, path_len)).unwrap_or("") };
    let magic = unsafe { std::slice::from_raw_parts(magic_ptr, magic_len.min(16)) };

    let file_type = detect_format(path, magic);
    // For WASM MVP: we can't easily return a string through the WASM boundary.
    // The host will use built-in Rust detection instead.
    // This function exists to verify WASM compilation works.

    if file_type.is_some() { 1 } else { 0 }
}

fn detect_format(path: &str, magic: &[u8]) -> Option<String> {
    // Check magic signatures first
    for (sig, name) in MAGIC_SIGS {
        if magic.starts_with(sig) {
            return Some(name.to_string());
        }
    }

    // Check extension
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    classify_by_ext(&ext).map(|s| s.to_string())
}

// Required by WASI
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() {}
