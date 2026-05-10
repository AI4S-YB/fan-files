/// FASTQ Context Interpreter
/// Infers assay type and sample info from FASTQ filename patterns

/// Common patterns:
/// - sample_R1.fastq.gz → paired-end, read 1
/// - sample_R2.fastq.gz → paired-end, read 2
/// - sample.fastq.gz → single-end
/// - sample_RNA-seq_rep1.fastq.gz → RNA-seq, replicate 1

/// Export: score confidence that this is a FASTQ file
#[unsafe(no_mangle)]
pub unsafe extern "C" fn score(path_ptr: *const u8, path_len: usize) -> f64 {
    let path = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(path_ptr, path_len)).unwrap_or("")
    };
    let lower = path.to_lowercase();

    if lower.contains(".fastq") || lower.contains(".fq") || lower.contains("fastq") {
        0.95
    } else {
        0.0
    }
}

/// Export: extract metadata
#[unsafe(no_mangle)]
pub unsafe extern "C" fn extract(_ctx_ptr: *const u8, _ctx_len: usize) -> i32 {
    // For WASM MVP: metadata extraction via serialized JSON context.
    // The host reads the file context (directory, siblings, etc.) and
    // passes it in. Full implementation requires proper WASM memory management.
    // Returns 0 for MVP.
    0
}

// Required by WASI
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() {}
