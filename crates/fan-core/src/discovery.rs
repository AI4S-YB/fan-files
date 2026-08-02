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
    OwnBio,      // Has bio files directly in this directory
    PropagatedBio, // Bio signal only from children (pure container)
    Bio,         // Legacy: kept for backward compat, treated as OwnBio
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
pub struct DatasetCandidate {
    pub path: String,
    pub dataset_type: String,
    pub species: Option<String>,
    pub confidence: String,
    /// LLM-classified role: "project_root" | "analysis_step" | "classification"
    /// analysis_step candidates are skipped in Phase C (they belong to parent project)
    pub candidate_role: Option<String>,
}

pub struct DiscoveryResult {
    pub targets: Vec<String>,
    pub skips: Vec<String>,
    pub uniform_dirs: Vec<UniformDir>,
    pub dataset_candidates: Vec<DatasetCandidate>,
}

/// Bio-relevant file extensions (not exhaustive — LLM handles the rest).
const BIO_EXTENSIONS: &[&str] = &[
    // ═══ Sequencing ═══
    "fastq", "fastq.gz", "fq", "fq.gz",
    "bam", "sam", "cram", "sra",
    // ═══ Sequences ═══
    "fa", "fasta", "fna", "faa", "ffn", "frn",
    "fa.gz", "fasta.gz",
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

/// Common compression suffixes in bioinformatics.
const COMPRESS_SUFFIXES: &[&str] = &["gz", "bz2", "bz", "xz", "zst", "zstd"];

/// Extension extraction (handles compound extensions like .fastq.gz, .fna.bz2, etc.).
pub fn light_file_extension(name: &str) -> String {
    let lower = name.to_lowercase();

    // Generic compression handling: .fna.gz, .fna.bz2, .faa.xz, etc.
    // If stripping a compression suffix reveals a known bio extension,
    // return the compound form "{bio_ext}.{comp}".
    for comp in COMPRESS_SUFFIXES {
        let suffix = format!(".{}", comp);
        if lower.ends_with(&suffix) {
            let inner = &lower[..lower.len() - suffix.len()];
            if let Some(pos) = inner.rfind('.') {
                let inner_ext = &inner[pos + 1..];
                if is_bio_ext(inner_ext) {
                    return format!("{}.{}", inner_ext, comp);
                }
            }
        }
    }

    // Explicit compound list for non-bio compressed files (e.g. .csv.gz, .txt.gz)
    // and other multi-part extensions.
    for compound in &[".fastq.gz", ".fq.gz", ".vcf.gz", ".gff.gz",
                       ".tsv.gz", ".csv.gz", ".txt.gz", ".tab.gz"] {
        if lower.ends_with(compound) { return compound[1..].to_string(); }
    }
    if let Some(pos) = name.rfind('.') {
        name[pos+1..].to_lowercase()
    } else {
        "(noext)".to_string()
    }
}

/// Check if an extension looks like a bioinformatics file format.
/// Also recognizes compressed variants (e.g. "fna.gz", "faa.bz2").
fn is_bio_ext(ext: &str) -> bool {
    if BIO_EXTENSIONS.contains(&ext) {
        return true;
    }
    // Strip compression suffix and check inner extension.
    // e.g. "fna.gz" → "fna", "faa.bz2" → "faa"
    for comp in COMPRESS_SUFFIXES {
        let suffix = format!(".{}", comp);
        if let Some(inner) = ext.strip_suffix(&suffix) {
            if BIO_EXTENSIONS.contains(&inner) {
                return true;
            }
        }
    }
    false
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
                if fp.has_bio_files {
                    // Has its own bio files AND bio children → OwnBio
                    fp.signal = DirSignal::OwnBio;
                } else {
                    // Bio signal only from children → pure container, not a dataset
                    fp.signal = DirSignal::PropagatedBio;
                }
            } else if fp.file_count > 0 {
                // Has files but none are bio → let LLM decide
                fp.signal = DirSignal::Unknown;
            } else {
                fp.signal = DirSignal::Empty;
            }
        }
    }

    // Step 4: Auto-include OwnBio dirs (exclude PropagatedBio containers).
    // PropagatedBio directories are pure containers with no own data files;
    // they should not be scan targets or dataset candidates.
    let auto_targets: Vec<String> = fingerprints.iter()
        .filter(|(_, fp)| fp.signal == DirSignal::OwnBio || fp.signal == DirSignal::Bio)
        .map(|(p, _)| p.clone())
        .collect();
    let container_count = fingerprints.values()
        .filter(|fp| fp.signal == DirSignal::PropagatedBio)
        .count();
    eprintln!("  Bottom-Up: {} BIO dirs auto-included ({} containers excluded)",
        auto_targets.len(), container_count);

    // Count ? dirs that need LLM decision
    let unknown_count = fingerprints.values()
        .filter(|fp| fp.signal == DirSignal::Unknown && fp.file_count > 0)
        .count();

    // Collect PropagatedBio containers that need LLM judgment.
    // Some PropagatedBio dirs are analysis pipeline projects (e.g. tie_sRNA/
    // with subdirs 01.raw, 02.clean) — they should be Datasets, not excluded.
    // We filter out only the most obvious non-analysis directories locally
    // (bare version numbers, obsolete markers). Everything else goes to LLM.
    let ambig_containers: Vec<(String, Vec<String>)> = fingerprints.iter()
        .filter(|(_, fp)| fp.signal == DirSignal::PropagatedBio && !fp.subdir_names.is_empty())
        .filter(|(_, fp)| {
            // Keep if any subdir doesn't look like a bare version tag or obsolete marker.
            // The LLM (guided by L2 rules) makes the final call.
            !fp.subdir_names.iter().all(|n| is_trivial_version_or_obsolete(n))
        })
        .map(|(path, fp)| (path.clone(), fp.subdir_names.clone()))
        .collect();
    if !ambig_containers.is_empty() {
        eprintln!("  Bottom-Up: {} PropagatedBio containers need LLM classification (others filtered locally)",
            ambig_containers.len());
    }

    let mut llm_targets: Vec<String> = Vec::new();
    let mut llm_dataset_candidates: Vec<DatasetCandidate> = Vec::new();

    // Use match (not ?) so LLM failure doesn't kill Phase A entirely.
    // auto_targets are always preserved regardless of LLM outcome.
    let llm_result: Result<_, Box<dyn std::error::Error>> = (|| {
        if unknown_count == 0 && ambig_containers.is_empty() {
            return Ok((vec![], vec![], vec![]));
        }
        eprintln!("  Bottom-Up: building compressed tree for LLM ({} ? dirs, {} ambiguous containers)...",
            unknown_count, ambig_containers.len());
        let prompt = build_bottom_up_prompt(&fingerprints, scan_root, base_depth);
        eprintln!("  Bottom-Up: prompt size = {} chars", prompt.len());
        if prompt.len() <= 50 {
            return Ok((vec![], vec![], vec![]));
        }
        llm_classify_bottom_up(llm_client, &prompt, scan_root, &ambig_containers)
    })();

    match llm_result {
        Ok((targets_from_llm, dataset_hints, container_roles)) => {
            llm_targets = targets_from_llm;
            llm_dataset_candidates = dataset_hints;
            for (path, role) in &container_roles {
                if role == "analysis_project" {
                    llm_dataset_candidates.push(DatasetCandidate {
                        path: path.clone(),
                        dataset_type: "other".to_string(),
                        species: None,
                        confidence: "low".to_string(),
                        candidate_role: None,
                    });
                }
            }
        }
        Err(e) => {
            eprintln!("  Bottom-Up: LLM call failed ({}). Proceeding with auto-detected targets.", e);
            // auto_targets are preserved — continue without LLM hints
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
    // Auto-generate dataset candidates from all targets (Phase A markers)
    // Each target IS a dataset candidate — Phase C will refine the type
    // Always ensure all auto-targets have candidates
    let existing_paths: std::collections::HashSet<String> = llm_dataset_candidates.iter()
        .map(|c| c.path.clone()).collect();
    for t in &targets {
        if !existing_paths.contains(t) {
            llm_dataset_candidates.push(DatasetCandidate {
                path: t.clone(),
                dataset_type: "other".to_string(),
                species: None,
                confidence: "low".to_string(),
                candidate_role: None,
            });
        }
    }

    Ok(DiscoveryResult { targets, skips, uniform_dirs, dataset_candidates: llm_dataset_candidates })

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
        DirSignal::OwnBio => "BIO",
        DirSignal::PropagatedBio => "BIO↑",
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

    // BIO directories (own or propagated): show summary, don't expand children
    if (fp.signal == DirSignal::Bio || fp.signal == DirSignal::OwnBio || fp.signal == DirSignal::PropagatedBio) && indent > 0 {
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

/// Minimal local filter: only skip directories whose subdirs are so obviously
/// non-analysis that sending to LLM would be wasteful.
/// This is intentionally conservative — it lets the LLM (with L2 rules) decide
/// ambiguous cases. Data-specific naming conventions belong in prompt-L2-user.md.
fn is_trivial_version_or_obsolete(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Bare version tag: "v1", "v2.0", "v1.1.3"
    (lower.starts_with('v')
        && lower.len() >= 2
        && lower[1..].chars().all(|c| c.is_ascii_digit() || c == '.' || c == '_'))
    // Obsolete marker: "obsolete", "obsolete_v1"
    || lower.starts_with("obsolete")
    // Empty or near-empty
    || lower.is_empty()
}

/// Send annotated tree to LLM for classification.
fn llm_classify_bottom_up(
    llm_client: &LlmClient,
    tree_prompt: &str,
    _scan_root: &str,
    ambig_containers: &[(String, Vec<String>)],
) -> Result<(Vec<String>, Vec<DatasetCandidate>, Vec<(String, String)>), Box<dyn std::error::Error>> {
    let mut full_prompt = format!(
        "你是生物信息学家。下面是压缩后的目录树，每个目录标注了文件组成。\n\
         用你的专业知识判断：哪些目录是\"数据集\"以及它是什么类型。\n\n\
         数据集 = 一组相关生物信息文件的集合，通常对应某类组学数据。\n\
         数据集类型参考（推理起点，不是硬规则）：\n\
         - genome: 大 .fa/.fasta + .fai/.gtf/.gff3 或 func_anno/ 子目录\n\
         - transcriptome: .fastq.gz 大量出现，子目录 raw/clean/mapping/expression\n\
         - variant: .vcf/.vcf.gz 出现，常与genome数据集共存\n\
         - epigenome: .bed + .bw + .narrowPeak/.broadPeak 组合\n\
         - metagenome: 大量 .fastq.gz + 无参考基因组特征\n\
         - germplasm: .final_table，种质编号模式\n\
         - proteome: .mzML/.mzXML 或蛋白 .fasta\n\
         - other: 无法归类时使用\n\n\
         判断时考虑：该目录自身文件 + 子目录结构。纯分类框架(0 files)不是数据集。\n\
         对每个候选，额外判断其角色(role)：\n\
         - project_root: 独立项目（含完整数据或分析步骤，自成一体）\n\
         - analysis_step: 项目内分析步骤（编号前缀如01.raw/02.clean/03.align，归入父项目）\n\
         - classification: 纯容器/分类目录（自身无核心数据文件，仅为组织子目录）\n\\n\
         物种信息能从目录名推断的，一并输出。\n\
         输出JSON: {{\"datasets\":[{{\"path\":\"路径\",\"dataset_type\":\"类型\",\"species\":\"物种\",\"confidence\":\"high\"}}],\n\
         \"scan_targets\":[{{\"path\":\"路径\"}}]}}\n\n{}",
        tree_prompt
    );

    // Append container classification task if needed
    if !ambig_containers.is_empty() {
        let container_lines: Vec<String> = ambig_containers.iter()
            .map(|(path, subs)| format!("  {} → 子目录: [{}]", path, subs.join(", ")))
            .collect();
        full_prompt.push_str(&format!(
            "\n\n---\n\n另外，以下目录自身没有文件，但在目录树中被标记为BIO↑。\n\
             请判断每个目录的子目录是否构成有序分析步骤/管线编排：\n\n\
             {}\n\n\
             判断规则:\n\
             - role=analysis_project: 子目录形成有序步骤（数字/字母编号、阶段/步骤序列等）\n\
             - role=taxonomic_container: 子目录是版本名、品种名、accession ID、或废弃标记，无步骤关系\n\
             在输出JSON中添加\"container_roles\":\n\
             {{\"container_roles\":[{{\"path\":\"路径\",\"role\":\"analysis_project|taxonomic_container\"}}]}}",
            container_lines.join("\n")
        ));
    }

    let body = serde_json::json!({
        "model": llm_client.config.model,
        "messages": [
            {"role": "system", "content": "你是生物信息学家。根据目录树和文件组成判断哪些目录是数据集及其类型。用专业知识推理，不被预设规则限制。"},
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
    let targets: Vec<String> = output["scan_targets"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    // Parse dataset candidates with role classification
    let candidates: Vec<DatasetCandidate> = output["datasets"].as_array()
        .map(|a| a.iter().filter_map(|v| {
            Some(DatasetCandidate {
                path: v["path"].as_str()?.to_string(),
                dataset_type: v["dataset_type"].as_str().unwrap_or("other").to_string(),
                species: v["species"].as_str().map(String::from),
                confidence: v["confidence"].as_str().unwrap_or("medium").to_string(),
                candidate_role: v["role"].as_str().map(String::from),
            })
        }).collect())
        .unwrap_or_default();
    // Parse container classification results
    let container_roles: Vec<(String, String)> = output["container_roles"].as_array()
        .map(|a| a.iter()
            .filter_map(|v| {
                let path = v["path"].as_str()?.to_string();
                let role = v["role"].as_str().unwrap_or("taxonomic_container").to_string();
                Some((path, role))
            })
            .collect())
        .unwrap_or_default();

    Ok((targets, candidates, container_roles))
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

    let targets: Vec<String> = output["scan_targets"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    let skips: Vec<String> = output["skips"].as_array()
        .map(|a| a.iter().filter_map(|v| v["path"].as_str().map(String::from)).collect())
        .unwrap_or_default();

    eprintln!("  Phase A: {} targets to scan, {} dirs to skip", targets.len(), skips.len());
    Ok(DiscoveryResult { targets, skips, uniform_dirs: Vec::new(), dataset_candidates: Vec::new() })
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
                        if let Some(arr) = output["scan_targets"].as_array() {
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
    Ok(DiscoveryResult { targets: all_targets, skips: all_skips, uniform_dirs: Vec::new(), dataset_candidates: Vec::new() })
}
