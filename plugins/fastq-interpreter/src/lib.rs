use serde::Serialize;

/// Static output buffer -- host reads from here after function calls.
static mut OUTPUT: [u8; 4096] = [0u8; 4096];

#[derive(Serialize)]
struct InterpretResult {
    assay_type: Option<String>,
    species: Option<String>,
    tags: Vec<String>,
}

fn infer_from_path(path: &str) -> InterpretResult {
    let lower = path.to_lowercase();

    let assay = if lower.contains("rnaseq") || lower.contains("rna-seq") || lower.contains("rna_seq")
    {
        Some("RNA-seq".into())
    } else if lower.contains("chipseq") || lower.contains("chip-seq") || lower.contains("chip_seq") {
        Some("ChIP-seq".into())
    } else if lower.contains("atac") || lower.contains("atac-seq") {
        Some("ATAC-seq".into())
    } else if lower.contains("wgs") || lower.contains("whole_genome") || lower.contains("dna-seq") {
        Some("WGS".into())
    } else if lower.contains("wgbs") || lower.contains("methylation") || lower.contains("bisulfite") {
        Some("WGBS".into())
    } else if lower.contains("scrna") || lower.contains("singlecell") || lower.contains("single_cell") || lower.contains("10x") {
        Some("scRNA-seq".into())
    } else {
        None
    };

    let species = if lower.contains("human") || lower.contains("hg38") || lower.contains("hg19") {
        Some("human".into())
    } else if lower.contains("mouse") || lower.contains("mm10") || lower.contains("mm39") {
        Some("mouse".into())
    } else if lower.contains("rat") || lower.contains("rattus") {
        Some("rat".into())
    } else if lower.contains("zebrafish") || lower.contains("danio") {
        Some("zebrafish".into())
    } else if lower.contains("drosophila") || lower.contains("dmel") {
        Some("fruit fly".into())
    } else if lower.contains("celegans") || lower.contains("elegans") {
        Some("C. elegans".into())
    } else if lower.contains("arabidopsis") || lower.contains("thaliana") {
        Some("Arabidopsis".into())
    } else if lower.contains("yeast") || lower.contains("cerevisiae") {
        Some("yeast".into())
    } else if lower.contains("ecoli") || lower.contains("e_coli") || lower.contains("k12") {
        Some("E. coli".into())
    } else {
        None
    };

    let mut tags = Vec::new();
    if lower.contains("_r1") || lower.contains("_r2") || lower.contains("_1.") || lower.contains("_2.") {
        tags.push("paired-end".into());
    } else if lower.contains(".fastq") || lower.contains(".fq") {
        tags.push("single-end".into());
    }

    InterpretResult {
        assay_type: assay,
        species,
        tags,
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn output_buffer() -> *const u8 {
    // Edition 2024: use raw pointer to avoid shared reference to static mut
    &raw const OUTPUT as *const u8
}

/// Export: score confidence that this file matches (0.0 - 1.0)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn score(path_ptr: *const u8, path_len: usize) -> f64 {
    let path = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(path_ptr, path_len))
    }
    .unwrap_or("");
    let lower = path.to_lowercase();

    if lower.contains(".fastq")
        || lower.contains(".fq")
        || lower.contains("fastq")
    {
        0.95
    } else {
        0.0
    }
}

/// Export: extract biological metadata from the file path.
/// Writes JSON result to the output buffer; returns output length, or 0 on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extract(path_ptr: *const u8, path_len: usize) -> i32 {
    let path = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(path_ptr, path_len))
    }
    .unwrap_or("");
    let result = infer_from_path(path);
    let json = serde_json::to_string(&result).unwrap_or_default();
    write_output(&json)
}

// no-op _start to satisfy wasm linker
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() {}
