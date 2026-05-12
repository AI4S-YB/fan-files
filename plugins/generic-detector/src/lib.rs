use serde::Serialize;

/// Static output buffer -- host reads from here after function calls.
/// Functions write JSON output to this buffer and return the length.
static mut OUTPUT: [u8; 4096] = [0u8; 4096];

#[derive(Serialize)]
struct DetectResult {
    file_type: String,
    mime: Option<String>,
}

/// Magic byte signatures for common bioinformatics file formats
static MAGIC_SIGS: &[(&[u8], &str)] = &[
    (b"BAM\x01", "BAM"),
    (b"CRAM", "CRAM"),
    (b"@HD\t", "SAM"),
    (b"##fileformat=VCF", "VCF"),
    (b"\x89HDF\r\n\x1a\n", "HDF5"),
    (b"HDF\r\n\x1a\n", "HDF5"),
    (b"\x1f\x8b", "gzip"),
    (b"BZh", "bzip2"),
    (b"PK\x03\x04", "zip"),
    (b"\x89PNG\r\n\x1a\n", "png"),
    (b"\x93NUMPY", "npy"),
    (b"RIFF", "riff"),
];

fn detect_format(path: &str, magic: &[u8]) -> Option<DetectResult> {
    // Check magic signatures first
    for (sig, name) in MAGIC_SIGS {
        if magic.len() >= sig.len() && &magic[..sig.len()] == *sig {
            return Some(DetectResult {
                file_type: name.to_string(),
                mime: Some("application/octet-stream".to_string()),
            });
        }
    }

    // Check text-based formats (FASTA and FASTQ/SAM)
    if !magic.is_empty() {
        if magic[0] == b'>' {
            return Some(DetectResult {
                file_type: "FASTA".to_string(),
                mime: Some("text/plain".to_string()),
            });
        }
        if magic[0] == b'@' {
            let header =
                std::str::from_utf8(magic.get(..20).unwrap_or(magic)).unwrap_or("");
            if header.starts_with("@HD")
                || header.starts_with("@SQ")
                || header.starts_with("@RG")
                || header.starts_with("@PG")
            {
                return Some(DetectResult {
                    file_type: "SAM".to_string(),
                    mime: Some("text/plain".to_string()),
                });
            }
            return Some(DetectResult {
                file_type: "FASTQ".to_string(),
                mime: Some("text/plain".to_string()),
            });
        }
    }

    // Extension-based fallback
    let ext = path
        .split('.')
        .last()
        .unwrap_or("")
        .to_lowercase();
    let file_type = match ext.as_str() {
        "fastq" | "fq" => "FASTQ",
        "fasta" | "fa" | "fna" | "faa" | "ffn" | "frn" => "FASTA",
        "bam" => "BAM",
        "sam" => "SAM",
        "cram" => "CRAM",
        "vcf" => "VCF",
        "bcf" => "BCF",
        "gff" | "gtf" | "gff3" => "GFF",
        "bed" => "BED",
        "bw" | "bigwig" => "bigWig",
        "bg" | "bedgraph" => "bedGraph",
        "h5" | "hdf5" | "h5ad" | "loom" => "HDF5",
        "csv" => "CSV",
        "tsv" | "tab" => "TSV",
        "txt" => "text",
        "json" => "JSON",
        "gz" => "gzip",
        "bz2" => "bzip2",
        "zip" => "zip",
        "yaml" | "yml" => "YAML",
        "rds" | "rda" => "RData",
        "mtx" => "MatrixMarket",
        _ => return None,
    };
    Some(DetectResult {
        file_type: file_type.to_string(),
        mime: Some("application/octet-stream".to_string()),
    })
}

/// Write JSON string to the static output buffer, null-terminated.
/// Returns the number of bytes written (excluding null terminator).
fn write_output(json: &str) -> i32 {
    let bytes = json.as_bytes();
    let len = bytes.len().min(4095);
    unsafe {
        OUTPUT[..len].copy_from_slice(&bytes[..len]);
        OUTPUT[len] = 0; // null terminate
    }
    len as i32
}

/// Export: return pointer to the static output buffer.
/// Host calls this to discover where results are written.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn output_buffer() -> *const u8 {
    // Edition 2024: use raw pointer to avoid shared reference to static mut
    &raw const OUTPUT as *const u8
}

/// Export: check if this detector can handle the file
#[unsafe(no_mangle)]
pub unsafe extern "C" fn can_handle(
    path_ptr: *const u8,
    path_len: usize,
    magic_ptr: *const u8,
    magic_len: usize,
) -> i32 {
    let path = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(path_ptr, path_len))
    }
    .unwrap_or("");
    let magic = unsafe { std::slice::from_raw_parts(magic_ptr, magic_len.min(16)) };
    if detect_format(path, magic).is_some() {
        1
    } else {
        0
    }
}

/// Export: detect format and write JSON result to output buffer.
/// Returns output length, or 0 if no match.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn detect(
    path_ptr: *const u8,
    path_len: usize,
    magic_ptr: *const u8,
    magic_len: usize,
) -> i32 {
    let path = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(path_ptr, path_len))
    }
    .unwrap_or("");
    let magic = unsafe { std::slice::from_raw_parts(magic_ptr, magic_len.min(16)) };

    match detect_format(path, magic) {
        Some(result) => {
            let json = serde_json::to_string(&result).unwrap_or_default();
            write_output(&json)
        }
        None => 0,
    }
}

// no-op _start to satisfy wasm linker
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() {}
