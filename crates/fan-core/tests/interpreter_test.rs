use fan_core::interpreter::InterpreterRegistry;
use fan_plugin_sdk::FileContext;

#[test]
fn test_fastq_rnaseq_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/projects/lung_cancer/rnaseq/sample_R1.fastq.gz".into(),
        siblings: vec![
            "/data/projects/lung_cancer/rnaseq/sample_R1.fastq.gz".into(),
            "/data/projects/lung_cancer/rnaseq/sample_R2.fastq.gz".into(),
        ],
        directory_tree: vec![
            "rnaseq".into(),
            "lung_cancer".into(),
            "projects".into(),
        ],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["FASTQ".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some(), "Expected best_interpretation to return BioMetadata");
    let meta = result.unwrap();
    assert_eq!(meta.assay_type, Some("RNA-seq".into()));
    assert_eq!(meta.project, Some("lung_cancer".into()));
    assert!(
        meta.tags.contains(&"paired-end".to_string()),
        "Expected paired-end tag, got tags: {:?}",
        meta.tags
    );
}

#[test]
fn test_fastq_single_end() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/wgs/sample.fastq.gz".into(),
        siblings: vec![],
        directory_tree: vec!["wgs".into(), "data".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["FASTQ".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.assay_type, Some("WGS".into()));
    assert!(
        meta.tags.contains(&"single-end".to_string()),
        "Expected single-end tag, got tags: {:?}",
        meta.tags
    );
}

#[test]
fn test_bam_chipseq_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/projects/leukemia/chipseq/H3K27ac/alignment.bam".into(),
        siblings: vec![
            "/data/projects/leukemia/chipseq/H3K27ac/alignment.bam".into(),
            "/data/projects/leukemia/chipseq/H3K27ac/peaks.bed".into(),
        ],
        directory_tree: vec![
            "H3K27ac".into(),
            "chipseq".into(),
            "leukemia".into(),
        ],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["BAM".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.assay_type, Some("ChIP-seq".into()));
    assert_eq!(meta.project, Some("leukemia".into()));
}

#[test]
fn test_human_species_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/human/hg38/rnaseq/sample.fastq.gz".into(),
        siblings: vec![],
        directory_tree: vec!["rnaseq".into(), "hg38".into(), "human".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["FASTQ".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.species, Some("human".into()));
}

#[test]
fn test_mouse_species_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/mus_musculus/mm10/atac-seq/sample.bam".into(),
        siblings: vec![],
        directory_tree: vec!["atac-seq".into(), "mm10".into(), "mus_musculus".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["BAM".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.species, Some("mouse".into()));
    assert_eq!(meta.assay_type, Some("ATAC-seq".into()));
}

#[test]
fn test_annotation_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/reference/human/genes.gtf".into(),
        siblings: vec![],
        directory_tree: vec!["human".into(), "reference".into(), "data".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["GTF".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.assay_type, Some("annotation".into()));
    assert_eq!(meta.species, Some("human".into()));
    assert!(
        meta.tags.contains(&"reference".to_string()),
        "Expected 'reference' tag for annotation files"
    );
}

#[test]
fn test_vcf_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/projects/cancer_study/wgs/calls.vcf.gz".into(),
        siblings: vec![],
        directory_tree: vec!["wgs".into(), "cancer_study".into(), "projects".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["VCF".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.assay_type, Some("WGS".into()));
    assert_eq!(meta.project, Some("cancer_study".into()));
}

#[test]
fn test_scrnaseq_detection() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/data/singlecell/10x_scrna/sample.h5".into(),
        siblings: vec![],
        directory_tree: vec!["10x_scrna".into(), "singlecell".into(), "data".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["HDF5".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    assert_eq!(meta.assay_type, Some("scRNA-seq".into()));
}

#[test]
fn test_generic_fallback() {
    let registry = InterpreterRegistry::new();
    // A file with no recognizable format tag but path clues
    let ctx = FileContext {
        file_path: "/data/results/some_tumor_report.csv".into(),
        siblings: vec![],
        directory_tree: vec!["results".into(), "data".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["CSV".into()],
        // NOTE: CSV doesn't have a specific interpreter, falls back to GenericInterpreter
    };

    let results = registry.interpret(&ctx);
    assert!(!results.is_empty());

    // The best score should come from GenericInterpreter
    let best = &results[0];
    assert_eq!(best.interpreter_name, "generic-interpreter");
    assert_eq!(best.score, 0.5);

    // Generic still tries to detect from path
    assert!(best.metadata.project.is_some() || best.metadata.assay_type.is_none());
}

#[test]
fn test_project_extraction_skips_generic_dirs() {
    let registry = InterpreterRegistry::new();
    let ctx = FileContext {
        file_path: "/home/user/data/projects/results/rnaseq/sample.fastq".into(),
        siblings: vec![],
        directory_tree: vec![
            "rnaseq".into(),
            "results".into(),
            "projects".into(),
            "data".into(),
        ],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["FASTQ".into()],
    };

    let result = registry.best_interpretation(&ctx, 0.3);
    assert!(result.is_some());
    let meta = result.unwrap();
    // "rnaseq", "results", "projects", "data" are all generic, so project should be None
    assert_eq!(meta.project, None);
    assert_eq!(meta.assay_type, Some("RNA-seq".into()));
}

#[test]
fn test_interpreter_score_zero_for_mismatch() {
    let registry = InterpreterRegistry::new();
    // A CSV file should get score 0 from FASTQ interpreter
    let ctx = FileContext {
        file_path: "/data/data.csv".into(),
        siblings: vec![],
        directory_tree: vec!["data".into()],
        metadata_files: vec![],
        file_header_b64: String::new(),
        format_tags: vec!["CSV".into()],
    };

    let results = registry.interpret(&ctx);
    // GenericInterpreter should be the only one above 0
    let best = &results[0];
    assert_eq!(best.interpreter_name, "generic-interpreter");
    assert!(results.iter().all(|r| r.score <= 0.5));
}
