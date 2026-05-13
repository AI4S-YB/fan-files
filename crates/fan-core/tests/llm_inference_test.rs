use fan_core::llm::prompt;

#[test]
fn test_directory_summary_generation() {
    let dirs = vec![
        (
            "/data/blastdb/".into(),
            101,
            vec!["arabidopsis_mrna.nhr".into(), "blast_names.json".into()],
        ),
        (
            "/data/fastq/apple_rnaseq_test/".into(),
            13,
            vec!["H_1_1.fq".into(), "meta.json".into()],
        ),
    ];
    let summary = prompt::build_directory_summary("/data", &dirs);
    assert!(summary.contains("blastdb"));
    assert!(summary.contains("apple_rnaseq_test"));
    assert!(summary.contains("101 files"));
    assert!(summary.contains("13 files"));
    assert!(summary.contains("H_1_1.fq"));
}

#[test]
fn test_llm_response_parsing() {
    let json = r#"{
        "projects": [
            {
                "name": "test_project",
                "dirs": ["/data/test/"],
                "assay_type": "RNA-seq",
                "species": "human",
                "species_confidence": "high",
                "summary": "test"
            }
        ],
        "relations": [
            {
                "project_a": "test_project",
                "project_b": "other_project",
                "relation": "same_species",
                "score": 0.9
            }
        ]
    }"#;
    let result = prompt::parse_llm_response(json).unwrap();
    assert_eq!(result.projects.len(), 1);
    assert_eq!(result.projects[0].name, "test_project");
    assert_eq!(result.projects[0].assay_type, Some("RNA-seq".into()));
    assert_eq!(result.projects[0].species, Some("human".into()));
    assert_eq!(result.relations.len(), 1);
    assert_eq!(result.relations[0].project_a, "test_project");
}

#[test]
fn test_llm_response_parsing_missing_relations() {
    // relations field is optional
    let json = r#"{
        "projects": [
            {"name": "p1", "dirs": ["/d1/"], "assay_type": null, "species": null, "summary": null}
        ]
    }"#;
    let result = prompt::parse_llm_response(json).unwrap();
    assert_eq!(result.projects.len(), 1);
    assert!(result.relations.is_empty());
}

#[test]
fn test_key_file_detection_in_summary() {
    let dirs = vec![
        (
            "/data/rnaseq/".into(),
            5,
            vec![
                "sample_R1.fastq.gz".into(),
                "sample_R2.fastq.gz".into(),
                "design.csv".into(),
            ],
        ),
    ];
    let summary = prompt::build_directory_summary("/data", &dirs);
    // FASTQ files should appear (they're key files)
    assert!(summary.contains("fastq.gz"));
}
