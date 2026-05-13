use fan_plugin_sdk::{BioMetadata, FileContext};
use std::path::Path;

/// Result from a context interpreter
pub struct InterpretationResult {
    pub interpreter_name: String,
    pub score: f64,
    pub metadata: BioMetadata,
}

/// Trait for context interpreters (mirrors what WASM plugins would implement)
pub trait ContextInterpreter {
    fn name(&self) -> &str;
    fn score(&self, ctx: &FileContext) -> f64;
    fn extract(&self, ctx: &FileContext) -> BioMetadata;
}

// ─── FASTQ Interpreter ───────────────────────────────────────

pub struct FastqInterpreter;

impl ContextInterpreter for FastqInterpreter {
    fn name(&self) -> &str {
        "fastq-interpreter"
    }

    fn score(&self, ctx: &FileContext) -> f64 {
        let path_lower = ctx.file_path.to_lowercase();
        if ctx.format_tags.iter().any(|t| t == "FASTQ")
            || path_lower.contains(".fastq")
            || path_lower.contains(".fq")
        {
            0.95
        } else {
            0.0
        }
    }

    fn extract(&self, ctx: &FileContext) -> BioMetadata {
        let mut meta = BioMetadata::default();
        let path_lower = ctx.file_path.to_lowercase();

        // Detect paired-end
        if path_lower.contains("_r1")
            || path_lower.contains("_r2")
            || path_lower.contains("_1.")
            || path_lower.contains("_2.")
        {
            meta.tags.push("paired-end".into());
        } else {
            meta.tags.push("single-end".into());
        }

        // Detect assay type from path
        let assay = detect_assay_from_path(&path_lower);
        if let Some(a) = assay {
            meta.assay_type = Some(a);
        }

        // Detect species from path
        let species = detect_species_from_path(&path_lower);
        if let Some(s) = species {
            meta.species = Some(s);
        }

        // Extract project from directory tree
        let project = extract_project(&ctx.directory_tree);
        if let Some(p) = project {
            meta.project = Some(p);
        }

        meta
    }
}

// ─── BAM/CRAM/SAM Interpreter ────────────────────────────────

pub struct BamInterpreter;

impl ContextInterpreter for BamInterpreter {
    fn name(&self) -> &str {
        "bam-interpreter"
    }

    fn score(&self, ctx: &FileContext) -> f64 {
        if ctx
            .format_tags
            .iter()
            .any(|t| matches!(t.as_str(), "BAM" | "CRAM" | "SAM"))
        {
            0.90
        } else {
            let path_lower = ctx.file_path.to_lowercase();
            if path_lower.ends_with(".bam")
                || path_lower.ends_with(".cram")
                || path_lower.ends_with(".sam")
            {
                0.90
            } else {
                0.0
            }
        }
    }

    fn extract(&self, ctx: &FileContext) -> BioMetadata {
        let mut meta = BioMetadata::default();
        let path_lower = ctx.file_path.to_lowercase();

        // Detect assay type from path
        let assay = detect_assay_from_path(&path_lower);
        if let Some(a) = assay {
            meta.assay_type = Some(a);
        }

        // Detect species
        let species = detect_species_from_path(&path_lower);
        if let Some(s) = species {
            meta.species = Some(s);
        }

        // Project from directory tree
        let project = extract_project(&ctx.directory_tree);
        if let Some(p) = project {
            meta.project = Some(p);
        }

        meta
    }
}

// ─── VCF/BCF Interpreter ─────────────────────────────────────

pub struct VcfInterpreter;

impl ContextInterpreter for VcfInterpreter {
    fn name(&self) -> &str {
        "vcf-interpreter"
    }

    fn score(&self, ctx: &FileContext) -> f64 {
        if ctx
            .format_tags
            .iter()
            .any(|t| matches!(t.as_str(), "VCF" | "BCF"))
        {
            0.90
        } else {
            let path_lower = ctx.file_path.to_lowercase();
            if path_lower.ends_with(".vcf")
                || path_lower.ends_with(".vcf.gz")
                || path_lower.ends_with(".bcf")
            {
                0.90
            } else {
                0.0
            }
        }
    }

    fn extract(&self, ctx: &FileContext) -> BioMetadata {
        let mut meta = BioMetadata::default();
        let path_lower = ctx.file_path.to_lowercase();

        let assay = detect_assay_from_path(&path_lower);
        if let Some(a) = assay {
            meta.assay_type = Some(a);
        }

        let species = detect_species_from_path(&path_lower);
        if let Some(s) = species {
            meta.species = Some(s);
        }

        let project = extract_project(&ctx.directory_tree);
        if let Some(p) = project {
            meta.project = Some(p);
        }

        meta
    }
}

// ─── HDF5/h5ad Interpreter (single-cell) ─────────────────────

pub struct Hdf5Interpreter;

impl ContextInterpreter for Hdf5Interpreter {
    fn name(&self) -> &str {
        "hdf5-interpreter"
    }

    fn score(&self, ctx: &FileContext) -> f64 {
        let path_lower = ctx.file_path.to_lowercase();
        if ctx
            .format_tags
            .iter()
            .any(|t| matches!(t.as_str(), "HDF5"))
        {
            0.85
        } else if path_lower.ends_with(".h5")
            || path_lower.ends_with(".h5ad")
            || path_lower.ends_with(".hdf5")
        {
            0.85
        } else {
            0.0
        }
    }

    fn extract(&self, ctx: &FileContext) -> BioMetadata {
        let mut meta = BioMetadata::default();
        let path_lower = ctx.file_path.to_lowercase();

        let assay = detect_assay_from_path(&path_lower);
        if let Some(a) = assay {
            meta.assay_type = Some(a);
        }

        let species = detect_species_from_path(&path_lower);
        if let Some(s) = species {
            meta.species = Some(s);
        }

        let project = extract_project(&ctx.directory_tree);
        if let Some(p) = project {
            meta.project = Some(p);
        }

        meta
    }
}

// ─── GFF/BED/Annotation Interpreter ──────────────────────────

pub struct AnnotationInterpreter;

impl ContextInterpreter for AnnotationInterpreter {
    fn name(&self) -> &str {
        "annotation-interpreter"
    }

    fn score(&self, ctx: &FileContext) -> f64 {
        let path_lower = ctx.file_path.to_lowercase();
        if ctx
            .format_tags
            .iter()
            .any(|t| matches!(t.as_str(), "GFF" | "GTF" | "BED"))
        {
            0.85
        } else if path_lower.ends_with(".gff")
            || path_lower.ends_with(".gtf")
            || path_lower.ends_with(".gff3")
            || path_lower.ends_with(".bed")
        {
            0.85
        } else if path_lower.contains("annotation") || path_lower.contains("genes") {
            0.6
        } else {
            0.0
        }
    }

    fn extract(&self, ctx: &FileContext) -> BioMetadata {
        let mut meta = BioMetadata::default();
        meta.assay_type = Some("annotation".into());
        meta.tags.push("reference".into());

        let species = detect_species_from_path(&ctx.file_path.to_lowercase());
        if let Some(s) = species {
            meta.species = Some(s);
        }

        let project = extract_project(&ctx.directory_tree);
        if let Some(p) = project {
            meta.project = Some(p);
        }

        meta
    }
}

// ─── Generic Interpreter (fallback) ──────────────────────────

pub struct GenericInterpreter;

impl ContextInterpreter for GenericInterpreter {
    fn name(&self) -> &str {
        "generic-interpreter"
    }

    fn score(&self, _ctx: &FileContext) -> f64 {
        0.5 // Always-on fallback — lower than specific interpreters
    }

    fn extract(&self, ctx: &FileContext) -> BioMetadata {
        let mut meta = BioMetadata::default();
        let path_lower = ctx.file_path.to_lowercase();

        let assay = detect_assay_from_path(&path_lower);
        if let Some(a) = assay {
            meta.assay_type = Some(a);
        }

        let species = detect_species_from_path(&path_lower);
        if let Some(s) = species {
            meta.species = Some(s);
        }

        let project = extract_project(&ctx.directory_tree);
        if let Some(p) = project {
            meta.project = Some(p);
        }

        meta
    }
}

// ─── Interpreter Registry ────────────────────────────────────

pub struct InterpreterRegistry {
    interpreters: Vec<Box<dyn ContextInterpreter + Send + Sync>>,
}

impl InterpreterRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            interpreters: Vec::new(),
        };
        // Order matters: more specific interpreters first
        registry.add(FastqInterpreter);
        registry.add(BamInterpreter);
        registry.add(VcfInterpreter);
        registry.add(Hdf5Interpreter);
        registry.add(AnnotationInterpreter);
        registry.add(GenericInterpreter); // Fallback, always last
        registry
    }

    pub fn add<I: ContextInterpreter + Send + Sync + 'static>(&mut self, interpreter: I) {
        self.interpreters.push(Box::new(interpreter));
    }

    /// Run all interpreters on a file context, return results sorted by score desc
    #[allow(dead_code)]
    pub fn interpret(&self, ctx: &FileContext) -> Vec<InterpretationResult> {
        let mut results: Vec<InterpretationResult> = self
            .interpreters
            .iter()
            .map(|interpreter| {
                let score = interpreter.score(ctx);
                let metadata = if score > 0.0 {
                    interpreter.extract(ctx)
                } else {
                    BioMetadata::default()
                };
                InterpretationResult {
                    interpreter_name: interpreter.name().to_string(),
                    score,
                    metadata,
                }
            })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Run interpreters and return the best metadata (highest score above threshold)
    pub fn best_interpretation(&self, ctx: &FileContext, threshold: f64) -> Option<BioMetadata> {
        let results = self.interpret(ctx);
        results
            .into_iter()
            .find(|r| r.score >= threshold)
            .map(|r| r.metadata)
    }
}

// ─── Public Helper Functions ─────────────────────────────────

/// List all sibling files in the same directory as `file_path`
pub fn list_siblings(file_path: &Path) -> Vec<String> {
    let mut siblings = Vec::new();
    if let Some(parent) = file_path.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                siblings.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    siblings
}

/// Build a directory tree (parent directory names going upward) for `levels` levels
pub fn directory_tree(file_path: &Path, levels: usize) -> Vec<String> {
    let mut tree = Vec::new();
    let mut current = file_path.parent();
    for _ in 0..levels {
        if let Some(dir) = current {
            tree.push(
                dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            );
            current = dir.parent();
        } else {
            break;
        }
    }
    tree
}

/// Find metadata files (design.csv, metadata.xlsx, README.md, etc.) near the file
pub fn find_metadata_files(file_path: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let metadata_names = [
        "design.csv",
        "design.tsv",
        "metadata.csv",
        "metadata.xlsx",
        "README.md",
        "readme.txt",
        "samples.csv",
        "samples.tsv",
        "config.yaml",
        "config.yml",
        "manifest.csv",
    ];
    if let Some(parent) = file_path.parent() {
        // Check current directory
        for name in &metadata_names {
            let path = parent.join(name);
            if path.exists() {
                found.push(path.to_string_lossy().to_string());
            }
        }
        // Check parent directory
        if let Some(grandparent) = parent.parent() {
            for name in &metadata_names {
                let path = grandparent.join(name);
                if path.exists() {
                    found.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
    found
}

// ─── Heuristic Functions ─────────────────────────────────────

fn detect_assay_from_path(path_lower: &str) -> Option<String> {
    let patterns: &[(&[&str], &str)] = &[
        (
            &[
                "rnaseq",
                "rna-seq",
                "rna_seq",
                "transcriptom",
                "mrna",
            ],
            "RNA-seq",
        ),
        (&["chipseq", "chip-seq", "chip_seq"], "ChIP-seq"),
        (&["atacseq", "atac-seq", "atac_seq"], "ATAC-seq"),
        (
            &["wgs", "whole_genome", "wholegenome", "dna-seq"],
            "WGS",
        ),
        (
            &["wgbs", "methylation", "bisulfite", "bs-seq"],
            "WGBS",
        ),
        (
            &[
                "singlecell",
                "single_cell",
                "scrna",
                "scrnaseq",
                "10x",
            ],
            "scRNA-seq",
        ),
        (
            &["scatac", "sc-atac", "single_cell_atac"],
            "scATAC-seq",
        ),
        (&["citeseq", "cite-seq"], "CITE-seq"),
        (&["hichip", "hi-c"], "Hi-C"),
        (
            &["clip", "clip-seq", "eclip", "rip-seq"],
            "CLIP-seq",
        ),
        (
            &["mirna", "small_rna", "smallrna", "srna"],
            "sRNA-seq",
        ),
        (&["riboseq", "ribo-seq"], "Ribo-seq"),
        (
            &["nanopore", "pacbio", "pb", "ont"],
            "long-read-seq",
        ),
    ];

    for (keywords, assay_name) in patterns {
        if keywords.iter().any(|kw| path_lower.contains(kw)) {
            return Some(assay_name.to_string());
        }
    }
    None
}

fn detect_species_from_path(path_lower: &str) -> Option<String> {
    let species_patterns: &[(&[&str], &str)] = &[
        (
            &["human", "homo_sapiens", "hsapiens", "hg38", "hg19"],
            "human",
        ),
        (
            &["mouse", "mus_musculus", "mmusculus", "mm10", "mm39"],
            "mouse",
        ),
        (&["rat", "rattus", "rnorvegicus"], "rat"),
        (
            &["zebrafish", "danio", "drerio", "danrer"],
            "zebrafish",
        ),
        (
            &["drosophila", "dmel", "melanogaster", "dm6"],
            "fruit fly",
        ),
        (
            &["celegans", "elegans", "worm"],
            "C. elegans",
        ),
        (
            &["arabidopsis", "thaliana", "athaliana"],
            "Arabidopsis",
        ),
        (
            &["yeast", "cerevisiae", "scerevisiae", "saccer3"],
            "yeast",
        ),
        (&["cattle", "cow", "btaurus", "bostau"], "cattle"),
        (&["pig", "sscrofa", "sus_scrofa"], "pig"),
        (&["chicken", "ggallus", "galgal"], "chicken"),
        (&["macaque", "rhesus", "rhemac"], "macaque"),
        (&["ecoli", "e_coli", "k12"], "E. coli"),
    ];

    for (keywords, species_name) in species_patterns {
        if keywords.iter().any(|kw| path_lower.contains(kw)) {
            return Some(species_name.to_string());
        }
    }
    None
}

fn extract_project(directory_tree: &[String]) -> Option<String> {
    // Look through directory tree for project-like directory names
    // The tree goes from file's parent up to root
    // Common pattern: /data/projects/<project_name>/...
    for dir in directory_tree.iter().rev() {
        let dir_lower = dir.to_lowercase();
        // Skip generic directory names
        if matches!(
            dir_lower.as_str(),
            "data"
                | "results"
                | "analysis"
                | "raw"
                | "processed"
                | "fastq"
                | "bam"
                | "vcf"
                | "rnaseq"
                | "chipseq"
                | "src"
                | "home"
                | "tmp"
                | "projects"
                | "users"
                | ""
                | "/"
                | "."
        ) {
            continue;
        }
        // If directory name looks like a project identifier, use it
        if dir.len() > 2 {
            return Some(dir.clone());
        }
    }
    None
}
