//! Progressive Discovery: Phase A — bottom-up fingerprint + LLM classification.
//!
//! Instead of depth-limited top-down walks (which miss deep data),
//! this module traverses ALL directories to find the deepest leaves first,
//! then propagates bio-signals upward from files to parents.
//! LLM receives a complete annotated tree where every directory shows
//! both its own files AND aggregated child signals.

use crate::llm::LlmClient;
use std::collections::HashMap;
use std::path::Path;

// ═══════════════════════════════════════════════════════════
// Bottom-Up Discovery (NEW — replaces recursive top-down)
// ═══════════════════════════════════════════════════════════

/// Per-directory fingerprint collected from readdir only (no file opens).
#[derive(Debug, Clone, Default)]
pub struct DirFingerprint {
    pub path: String,
    pub name: String,
    pub depth: usize,
    /// Direct file extension distribution (top 5)
    pub extensions: Vec<(String, usize)>,
    /// Total direct files
    pub file_count: usize,
    /// Sample file names (up to 8) for LLM context
    pub sample_files: Vec<String>,
    /// Direct subdirectory names
    pub subdir_names: Vec<String>,
    /// Count of subdirs with bio signal
    pub child_bio: usize,
    /// Total subdir count
    pub child_total: usize,
    /// This directory itself contains bio-extension files
    pub has_bio_files: bool,
    /// Final signal (own files OR propagated from children)
    pub signal: DirSignal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DirSignal {
    Bio,         // Contains or propagates bio signal
    Noise,       // Explicitly noise (.git, node_modules, etc.)
    Empty,       // No files, no bio children
    Unknown,     // Needs LLM to decide
}

impl Default for DirSignal {
    fn default() -> Self { DirSignal::Unknown }
}

/// Uniform-extension directory: Phase A detected directory with
/// >100 files all sharing the same extension. Phase B can skip
/// per-file open/read and batch-insert with shared format info.
#[derive(Debug, Clone)]
pub struct UniformDir {
    pub path: String,
    pub extension: String,
    pub file_count: usize,
    /// Sample absolute file paths for format detection (up to 5)
    pub sample_paths: Vec<String>,
}

/// Result of bottom-up discovery: targets to scan, dirs to skip,
/// and uniform-extension dirs for fast batch indexing.
pub struct DiscoveryResult {
    pub targets: Vec<String>,
    pub skips: Vec<String>,
    pub uniform_dirs: Vec<UniformDir>,
}

/// Bio-relevant file extensions (not exhaustive — LLM handles the rest).
const BIO_EXTENSIONS: &[&str] = &[
    // ═══ Sequencing ═══
    "fastq", "fastq.gz", "fq", "fq.gz",
    "bam", "sam", "cram", "sra",
    // ═══ Sequences ═══
    "fa", "fasta", "fna", "faa", "ffn", "frn", "fa.gz", "fasta.gz",
    "cds", "pep", "rna", "dna",
    // ═══ Alignment indices ═══
    "bai", "crai", "fai", "tbi", "csi", "paf",
    // ═══ Variants ═══
    "vcf", "vcf.gz", "bcf", "gvcf", "gvcf.gz",
    // ═══ Population genetics ═══
    "ped", "map", "bim", "fam", "tped", "tfam", "hmp",
    "eigenvec", "eigenval",
    // ═══ Annotation ═══
    "gff", "gtf", "gff3", "gff.gz", "gff3.gz", "gtf.gz",
    "bed", "bed.gz",
    // ═══ Genome browser tracks ═══
    "bw", "bigwig", "bigBed", "bedgraph", "bg",
    // ═══ Epigenomics ═══
    "narrowPeak", "broadPeak", "tagAlign",
    // ═══ GenBank / EMBL ═══
    "gb", "gbk", "embl",
    // ═══ Expression ═══
    "counts", "count", "fpkm", "rpkm", "tpm",
    // ═══ Single cell / matrix ═══
    "h5", "hdf5", "h5ad", "mtx", "rds", "loom",
    // ═══ Phylogenetics ═══
    "nwk", "newick", "tree", "tre", "nex", "nexus",
    "phy", "phylip", "maf", "aln", "stockholm",
    "nhx", "phyloxml",
    // ═══ HMMER ═══
    "hmm", "sto",
    // ═══ Synteny ═══
    "anchors", "collinearity",
    // ═══ Protein structure ═══
    "pdb", "pdbqt", "mmcif", "cif", "sdf", "mol", "mol2",
    // ═══ Metabolomics ═══
    "mzML", "mzXML", "mzData", "nmrML",
    // ═══ Microbiome ═══
    "biom", "qza", "qzv",
    // ═══ Genome assembly ═══
    "agp", "chain", "net",
    // ═══ Pathway ═══
    "gmt", "gmx", "kgml", "gpml", "obo",
    // ═══ Genotyping arrays ═══
    "idat", "gtc", "cel",
    // ═══ Phenomics / breeding ═══
    "phen", "blup", "gebv",
];

/// Known noise patterns (directories to always skip).
const NOISE_PATTERNS: &[&str] = &[
    ".git", "node_modules", "__pycache__", ".DS_Store",
    "__MACOSX", ".idea", ".vscode", "target",
];

/// Extension extraction (handles compound extensions like .fastq.gz).
pub fn light_file_extension(name: &str) -> String {
    let lower = name.to_lowercase();
    for compound in &[".fastq.gz", ".fq.gz", ".vcf.gz", ".gff.gz",
                       ".tsv.gz", ".csv.gz", ".txt.gz", ".tab.gz", ".fa.gz"] {
        if lower.ends_with(compound) { return compound[1..].to_string(); }
    }
    if let Some(pos) = name.rfind('.') {
        name[pos+1..].to_lowercase()
    } else {
        "(noext)".to_string()
    }
}

/// Check if an extension looks like a bioinformatics file format.
fn is_bio_ext(ext: &str) -> bool {
    BIO_EXTENSIONS.contains(&ext)
}

/// Check if a directory name matches known noise patterns.
fn is_noise_dir(name: &str) -> bool {
    NOISE_PATTERNS.contains(&name)
}

// ═══════════════════════════════════════════════════════════
// Step 1: Find ALL directories (fast: directory-only traversal)
// ═══════════════════════════════════════════════════════════

/// Recursively collect all directory paths under `root`, with depth info.
/// Only calls readdir — no file open, no stat per file.
/// Returns vec sorted by depth descending (deepest first).
pub fn find_all_dirs(root: &str) -> Vec<(String, usize)> {
    let mut dirs: Vec<(String, usize)> = Vec::new();
    let base_depth = Path::new(root).components().count();
    collect_dirs_recursive(root, base_depth, &mut dirs);
    // Sort deepest first for bottom-up processing
    dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    dirs
}

fn collect_dirs_recursive(path: &str, depth: usize, dirs: &mut Vec<(String, usize)>) {
    let dir_path = Path::new(path);
    let name = dir_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Skip noise dirs immediately
    if is_noise_dir(&name) { return; }

    let rel_depth = depth.saturating_sub(
        Path::new(path).components().count().saturating_sub(1)
    );
    dirs.push((path.to_string(), depth));

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_type().map_or(false, |t| t.is_dir()) {
                collect_dirs_recursive(
                    &entry.path().to_string_lossy(),
                    depth + 1,
                    dirs,
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Step 2: Fingerprint each directory
// ═══════════════════════════════════════════════════════════

/// Read directory entries (files + subdirs) and build fingerprint.
/// Only does readdir — no file open, no content read.
pub fn fingerprint_dir(path: &str) -> DirFingerprint {
    let dir_path = Path::new(path);
    let name = dir_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut fp = DirFingerprint {
        path: path.to_string(),
        name,
        depth: 0, // filled later
        ..Default::default()
    };

    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let mut sample_files: Vec<String> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let ft = entry.file_type().ok();
            let fname = entry.file_name().to_string_lossy().to_string();

            if ft.as_ref().map_or(false, |t| t.is_dir()) {
                if !is_noise_dir(&fname) {
                    fp.subdir_names.push(fname);
                }
            } else if ft.as_ref().map_or(false, |t| t.is_file()) {
                fp.file_count += 1;
                let ext = light_file_extension(&fname);
                *ext_counts.entry(ext).or_insert(0) += 1;
                // Keep up to 8 sample file names
                if sample_files.len() < 8 {
                    sample_files.push(fname);
                }
            }
        }
    }

    // Check if this directory has bio files
    fp.has_bio_files = ext_counts.keys().any(|e| is_bio_ext(e));

    // Top 5 extensions
    let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
    ext_vec.sort_by(|a, b| b.1.cmp(&a.1));
    ext_vec.truncate(5);
    fp.extensions = ext_vec;
    fp.sample_files = sample_files;
    fp.child_total = fp.subdir_names.len();

    // Initial signal
    fp.signal = if fp.has_bio_files {
        DirSignal::Bio
    } else if fp.file_count == 0 && fp.subdir_names.is_empty() {
        DirSignal::Empty
    } else {
        DirSignal::Unknown
    };

    fp
}

// ═══════════════════════════════════════════════════════════
// Step 3: Bottom-up signal propagation
// ═══════════════════════════════════════════════════════════

/// Run bottom-up discovery: find all dirs → fingerprint → propagate → LLM classify.
pub fn run_bottom_up_discovery(
    scan_root: &str,
    llm_client: &LlmClient,
) -> Result<DiscoveryResult, Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        return Err("LLM not configured".into());
    }

    eprintln!("  Bottom-Up: finding all directories...");
    let all_dirs = find_all_dirs(scan_root);
    eprintln!("  Bottom-Up: {} directories found", all_dirs.len());

    // Step 2: Fingerprint all dirs, deepest first
    eprintln!("  Bottom-Up: fingerprinting directories...");
    let mut fingerprints: HashMap<String, DirFingerprint> = HashMap::new();

    for (path, depth) in &all_dirs {
        let mut fp = fingerprint_dir(path);
        fp.depth = *depth;
        fingerprints.insert(path.clone(), fp);
    }

    // Step 3: Bottom-up signal propagation
    eprintln!("  Bottom-Up: propagating signals upward...");
    let root_path = Path::new(scan_root);
    let base_depth = root_path.components().count();

    // Process deepest first (all_dirs is already sorted by depth desc)
    for (path, _depth) in &all_dirs {
        // Find parent
        if let Some(parent) = Path::new(path).parent() {
            let parent_path = parent.to_string_lossy().to_string();
            // Read child signal first (immutable borrow)
            let child_is_bio = fingerprints.get(path)
                .map_or(false, |fp| fp.signal == DirSignal::Bio);
            // Then mutate parent (mutable borrow)
            if child_is_bio {
                if let Some(parent_fp) = fingerprints.get_mut(&parent_path) {
                    parent_fp.child_bio += 1;
                }
            }
        }
    }

    // Update parent signals after all child counts are collected
    for (_path, fp) in fingerprints.iter_mut() {
        if fp.signal == DirSignal::Unknown {
            if fp.child_bio > 0 {
                // Signal propagated from bio children
                fp.signal = DirSignal::Bio;
            } else if fp.file_count > 0 {
                // Has files but none are bio → let LLM decide
                fp.signal = DirSignal::Unknown;
            } else {
                fp.signal = DirSignal::Empty;
            }
        }
    }

    // Step 4: Auto-include all BIO dirs + send ? dirs to LLM
    let auto_targets: Vec<String> = fingerprints.iter()
        .filter(|(_, fp)| fp.signal == DirSignal::Bio)
        .map(|(p, _)| p.clone())
        .collect();
    eprintln!("  Bottom-Up: {} BIO dirs auto-included", auto_targets.len());

    // Count ? dirs that need LLM decision
    let unknown_count = fingerprints.values()
        .filter(|fp| fp.signal == DirSignal::Unknown && fp.file_count > 0)
        .count();

    let mut llm_targets: Vec<String> = Vec::new();
    if unknown_count > 0 {
        // Build compressed annotated tree for LLM
        eprintln!("  Bottom-Up: building compressed tree for LLM ({} ? dirs)...", unknown_count);
        let prompt = build_bottom_up_prompt(&fingerprints, scan_root, base_depth);
        eprintln!("  Bottom-Up: prompt size = {} chars", prompt.len());

        if prompt.len() > 500 {
            llm_targets = llm_classify_bottom_up(llm_client, &prompt, scan_root)?;
        }
    }

    let mut targets = auto_targets;
    targets.extend(llm_targets);
    targets.sort();
    targets.dedup();

    // Determine skipped dirs
    let all_paths: Vec<String> = fingerprints.keys().cloned().collect();
    let skips: Vec<String> = all_paths
        .into_iter()
        .filter(|p| !targets.iter().any(|t| p.starts_with(t)))
        .collect();

    // Detect uniform-extension dirs for Phase B fast-path
    const UNIFORM_MIN_FILES: usize = 100;
    let mut uniform_dirs: Vec<UniformDir> = Vec::new();
    for (path, fp) in &fingerprints {
        if fp.file_count >= UNIFORM_MIN_FILES
            && fp.extensions.len() == 1
            && !fp.subdir_names.is_empty() == false  // leaf dir (no subdirs)
        {
            // Collect up to 5 sample absolute paths
            let sample_paths: Vec<String> = fp.sample_files.iter()
                .take(5)
                .map(|f| format!("{}/{}", path.trim_end_matches('/'), f))
                .collect();
            if !sample_paths.is_empty() {
                let ext = fp.extensions[0].0.clone();
                uniform_dirs.push(UniformDir {
                    path: path.clone(),
                    extension: ext,
                    file_count: fp.file_count,
                    sample_paths,
                });
            }
        }
    }
    if !uniform_dirs.is_empty() {
        eprintln!("  Bottom-Up: {} uniform-extension dirs (Phase B fast-path)", uniform_dirs.len());
    }

    eprintln!(
        "  Bottom-Up complete: {} targets, {} skipped",
        targets.len(),
        skips.len()
    );
    Ok(DiscoveryResult { targets, skips, uniform_dirs })
}

/// Build a condensed annotated tree prompt from bottom-up fingerprints.
fn build_bottom_up_prompt(
    fingerprints: &HashMap<String, DirFingerprint>,
    root: &str,
    base_depth: usize,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("目录树 (每目录含信号标注):\n".to_string());
    build_prompt_recursive(fingerprints, root, base_depth, 0, &mut lines);
    lines.join("\n")
}

fn build_prompt_recursive(
    fingerprints: &HashMap<String, DirFingerprint>,
    path: &str,
    _base_depth: usize,
    indent: usize,
    lines: &mut Vec<String>,
) {
    let fp = match fingerprints.get(path) {
        Some(f) => f,
        None => return,
    };

    let prefix = "  ".repeat(indent);
    let signal_icon = match fp.signal {
        DirSignal::Bio => "BIO",
        DirSignal::Noise => "NOISE",
        DirSignal::Empty => "EMPTY",
        DirSignal::Unknown => "?",
    };

    let ext_str: Vec<String> = fp.extensions.iter()
        .map(|(e, c)| format!("{}×{}", e, c))
        .collect();

    let child_info = if fp.child_total > 0 {
        format!(" | sub:{} bio:{}", fp.child_total, fp.child_bio)
    } else {
        String::new()
    };

    // BIO directories: show summary, don't expand children (saves prompt space)
    if fp.signal == DirSignal::Bio && indent > 0 {
        lines.push(format!(
            "{}{} {}/ (auto:{} files{} sub:{}/{})",
            prefix, signal_icon, fp.name,
            fp.file_count,
            if ext_str.is_empty() { String::new() } else { format!(" [{}]", ext_str.join(",")) },
            fp.child_bio, fp.child_total,
        ));
        return;
    }

    // EMPTY dirs: skip silently if deep, show one line if shallow
    if fp.signal == DirSignal::Empty && indent > 1 {
        return;
    }

    let ext_display = if ext_str.is_empty() { String::from("none") } else { ext_str.join(",") };

    lines.push(format!(
        "{}{} {}/ (f:{} {}{}){}",
        prefix, signal_icon, fp.name, fp.file_count,
        ext_display,
        if fp.subdir_names.is_empty() { String::new() } else { format!(" sub:{}", fp.subdir_names.len()) },
        child_info,
    ));

    // Only recurse into Unknown dirs or root-level
    if fp.signal != DirSignal::Bio {
        for sub_name in &fp.subdir_names {
            let sub_path = format!("{}/{}", path.trim_end_matches('/'), sub_name);
            if fingerprints.contains_key(&sub_path) {
                build_prompt_recursive(fingerprints, &sub_path, _base_depth, indent + 1, lines);
            }
        }
    }
}

/// Send annotated tree to LLM for classification.
fn llm_classify_bottom_up(
    llm_client: &LlmClient,
    tree_prompt: &str,
    _scan_root: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let full_prompt = format!(
        "你是生物信息数据管理助手。下面是压缩后的目录树:\n\
         - BIO = 有生信文件/传播信号 → 已自动纳入扫描，子目录省略\n\
         - ?   = 不确定 → 需要你根据目录名+文件后缀+上下文判断\n\
         - EMPTY = 无文件无子目录 → 已自动跳过\n\
         - NOISE = 已知噪音(.git等) → 已自动跳过\n\n\
         你的任务:\n\
         1. 判断 ? 目录是否为生信项目/分析步骤/分类框架/噪音\n\
         2. 识别项目边界(分析步骤型/物种分类型/子项目型/泛基因组型)\n\
         3. 每个 ? 目录只有一行(子目录已省略)，你需要根据目录名和文件后缀判断\n\
         输出JSON: {{\"targets\":[{{\"path\":\"完整路径\"}}], \"project_roots\":[{{\"path\":\"路径\"}}]}}\n\
         (BIO目录不需列出，已自动全部纳入)\n\n{}",
        tree_prompt
    );

    let body = serde_json::json!({
        "model": llm_client.config.model,
        "messages": [
            {"role": "system", "content": "你是生物信息数据管理助手。根据带信号标注的目录树判断哪些目录需要纳入扫描。🟢=有生信信号，直接纳入。🟡=需根据上下文判断。"},
            {"role": "user", "content": full_prompt}
        ],
        "response_format": {"type": "json_object"},
        "temperature": 0.1,
        "max_tokens": 8192
    });

    eprintln!("  Bottom-Up: asking LLM to classify annotated tree...");
    let response: serde_json::Value = crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 3)?;
    let content = response["choices"][0]["message"]["content"]
        .as_str().ok_or("No content")?;

    let output: serde_json::Value = serde_json::from_str(content)?;
    let targets: Vec<String> = output["targets"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    Ok(targets)
}

// ═══════════════════════════════════════════════════════════
// Legacy top-down API (kept for backward compat)
// ═══════════════════════════════════════════════════════════

/// A lightweight directory node (original top-down struct).
#[derive(Debug, Clone)]
pub struct LightDirNode {
    pub name: String,
    pub path: String,
    pub file_count: usize,
    pub subdir_count: usize,
    pub extensions: Vec<(String, usize)>,
    pub subdirs: Vec<LightDirNode>,
}

/// Walk directory tree to given depth (original top-down).
pub fn lightweight_walk(root: &str, depth: u32) -> LightDirNode {
    let root_path = Path::new(root);
    let name = root_path
        .file_name().map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string());

    let mut node = LightDirNode {
        name: name.clone(),
        path: root.to_string(),
        file_count: 0,
        subdir_count: 0,
        extensions: Vec::new(),
        subdirs: Vec::new(),
    };

    if depth == 0 { return node; }

    let mut ext_counts: HashMap<String, usize> = HashMap::new();

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let ft = entry.file_type().ok();
            if ft.as_ref().map_or(false, |t| t.is_dir()) {
                node.subdir_count += 1;
                if depth > 1 {
                    let sub_path = entry.path().to_string_lossy().to_string();
                    let child = lightweight_walk(&sub_path, depth - 1);
                    if child.file_count > 0 || child.subdir_count > 0 {
                        node.subdirs.push(child);
                    }
                }
            } else if ft.as_ref().map_or(false, |t| t.is_file()) {
                node.file_count += 1;
                let fname = entry.file_name().to_string_lossy().to_string();
                let ext = light_file_extension(&fname);
                *ext_counts.entry(ext).or_insert(0) += 1;
            }
        }
    }

    let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
    ext_vec.sort_by(|a, b| b.1.cmp(&a.1));
    ext_vec.truncate(5);
    node.extensions = ext_vec;

    node.subdirs.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    node
}

/// Convert light tree to prompt (original).
pub fn light_tree_to_prompt(root: &LightDirNode, indent: usize) -> String {
    let mut lines = Vec::new();
    let prefix = "  ".repeat(indent);
    let ext_summary: Vec<String> = root.extensions.iter()
        .map(|(e, c)| format!("{}×{}", e, c))
        .collect();

    if root.subdirs.is_empty() {
        lines.push(format!(
            "{}📁 {}/  ({} files: {})",
            prefix, root.name, root.file_count, ext_summary.join(", ")
        ));
    } else {
        lines.push(format!(
            "{}📁 {}/  ({} files: {}, {} subdirs)",
            prefix, root.name, root.file_count, ext_summary.join(", "), root.subdirs.len()
        ));
        for child in &root.subdirs {
            lines.push(light_tree_to_prompt(child, indent + 1));
        }
    }
    lines.join("\n")
}

/// Original Phase A (kept for `fan-files discover` without --deep).
pub fn run_phase_a(
    scan_root: &str,
    llm_client: &LlmClient,
) -> Result<DiscoveryResult, Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        return Err("LLM not configured".into());
    }

    eprintln!("  Phase A: lightweight directory walk (depth 3)...");
    let tree = lightweight_walk(scan_root, 3);
    let prompt = light_tree_to_prompt(&tree, 0);
    eprintln!("  Phase A: tree built ({} chars prompt)", prompt.len());

    let full_prompt = format!(
        "你是生物信息数据管理助手。下面是一个目录树，每个目录显示了文件扩展名分布(来自轻量扫描，只统计了目录条目)。\n\n\
         对每个子目录，根据目录名和扩展名分布判断它的身份和扫描决策:\n\
         - project_root(独立研究项目) → scan，它下面的所有子目录都要扫(程序安装目录如Bioconductor/conda/envs除外)\n\
         - analysis_step(项目内的分析步骤，如01.raw/02.clean/03.miRNA) → scan，即使它自己没有典型的生信文件\n\
         - classification(分类目录，比项目大) → deeper，需要往下展开再判断\n\
         - noise(噪音/工具/缓存/安装目录) → skip\n\n\
         输出JSON: {{\"targets\":[{{\"path\":\"子目录路径\"}}], \"skips\":[{{\"path\":\"路径\"}}], \"deeper\":[{{\"path\":\"路径\"}}]}}\n\n{}",
        prompt
    );

    let system_prompt = "你是生物信息数据管理助手。根据目录名和扩展名分布判断目录身份(project_root/analysis_step/classification/noise)和扫描决策。";

    let body = serde_json::json!({
        "model": llm_client.config.model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": full_prompt}
        ],
        "response_format": {"type": "json_object"},
        "temperature": 0.1,
        "max_tokens": 8192
    });

    eprintln!("  Phase A: asking LLM to classify...");
    let response: serde_json::Value = crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 3)?;
    let content = response["choices"][0]["message"]["content"]
        .as_str().ok_or("No content")?;

    let output: serde_json::Value = serde_json::from_str(content)?;

    let targets: Vec<String> = output["targets"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    let skips: Vec<String> = output["skips"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    eprintln!("  Phase A: {} targets to scan, {} dirs to skip", targets.len(), skips.len());
    Ok(DiscoveryResult { targets, skips, uniform_dirs: Vec::new() })
}

/// Recursive Phase A (original, kept for backward compat).
pub fn run_recursive_phase_a(
    scan_root: &str,
    llm_client: &LlmClient,
    max_depth: u32,
) -> Result<DiscoveryResult, Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        return Err("LLM not configured".into());
    }

    let mut all_targets: Vec<String> = Vec::new();
    let mut all_skips: Vec<String> = Vec::new();
    let mut current_roots: Vec<String> = vec![scan_root.to_string()];
    let mut round = 1;

    while !current_roots.is_empty() && round <= 3 {
        let walk_depth: u32 = 3;

        eprintln!("  Phase A Round {}: {} root(s), depth {}...", round, current_roots.len(), walk_depth);

        let mut next_roots: Vec<String> = Vec::new();

        for root in &current_roots {
            let tree = lightweight_walk(root, walk_depth);
            let prompt = light_tree_to_prompt(&tree, 0);
            if prompt.len() < 50 { continue; }

            let full_prompt = format!(
                "你是生物信息数据管理助手。下面是一个子目录树(深层展开)。\n\
                 根据目录名和扩展名分布判断每个子目录身份:\n\
                 project_root → scan | analysis_step → scan | classification → deeper | noise → skip\n\
                 输出JSON: {{\"targets\":[{{\"path\":\"路径\"}}], \"skips\":[{{\"path\":\"路径\"}}], \"deeper\":[{{\"path\":\"路径\"}}]}}\n\n{}",
                prompt
            );

            let body = serde_json::json!({
                "model": llm_client.config.model,
                "messages": [
                    {"role": "system", "content": "你是生物信息数据管理助手。根据目录名和扩展名分布判断目录身份: project_root/analysis_step/classification/noise。"},
                    {"role": "user", "content": full_prompt}
                ],
                "response_format": {"type": "json_object"},
                "temperature": 0.1,
                "max_tokens": 4096
            });

            match crate::llm::llm_api_call_with_retry(&llm_client.config, &body, 2) {
                Ok(response) => {
                    let content = response["choices"][0]["message"]["content"].as_str().unwrap_or("");
                    if let Ok(output) = serde_json::from_str::<serde_json::Value>(content) {
                        if let Some(arr) = output["targets"].as_array() {
                            for v in arr {
                                if let Some(p) = v["path"].as_str() {
                                    let abs = if p.starts_with('/') { p.to_string() }
                                        else { format!("{}/{}", root.trim_end_matches('/'), p.trim_start_matches('/')) };
                                    all_targets.push(abs);
                                }
                            }
                        }
                        if let Some(arr) = output["skips"].as_array() {
                            for v in arr {
                                if let Some(p) = v["path"].as_str() {
                                    all_skips.push(p.to_string());
                                }
                            }
                        }
                        if let Some(arr) = output["deeper"].as_array() {
                            for v in arr {
                                if let Some(p) = v["path"].as_str() {
                                    let abs = if p.starts_with('/') { p.to_string() }
                                        else { format!("{}/{}", root.trim_end_matches('/'), p.trim_start_matches('/')) };
                                    next_roots.push(abs);
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("  Round {} LLM failed: {}", round, e),
            }
        }

        eprintln!("  Round {}: {} targets, {} deeper", round, all_targets.len(), next_roots.len());
        current_roots = next_roots;
        round += 1;

        if round as u32 * 3 > max_depth { break; }
    }

    all_targets.sort();
    all_targets.dedup();
    all_skips.sort();
    all_skips.dedup();

    eprintln!("  Recursive Phase A complete: {} targets, {} skipped", all_targets.len(), all_skips.len());
    Ok(DiscoveryResult { targets: all_targets, skips: all_skips, uniform_dirs: Vec::new() })
}
