use fan_core::config::Config;
use fan_core::scanner::Scanner;
use std::io::Write;

#[test]
fn test_scanner_discovers_files() {
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("test.fastq");
    std::fs::File::create(&file_path).unwrap()
        .write_all(b"@SEQ_ID\nACGT\n+\nIIII\n").unwrap();

    let scanner = Scanner::new(
        vec![tmp.path().to_string_lossy().to_string()],
        vec![],
        "test".to_string(),
    );

    let files: Vec<_> = scanner.scan().collect();
    assert!(!files.is_empty());
    assert!(files.iter().any(|f| f.path == file_path));
    assert_eq!(files[0].source_server, "test");
    assert_eq!(files[0].mime_type, "application/octet-stream"); // .fastq is not recognized by mime_guess
}

#[test]
fn test_scanner_excludes_patterns() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::File::create(tmp.path().join("data.txt")).unwrap().write_all(b"hello").unwrap();
    std::fs::File::create(tmp.path().join("data.tmp")).unwrap().write_all(b"temp").unwrap();

    let scanner = Scanner::new(
        vec![tmp.path().to_string_lossy().to_string()],
        vec!["*.tmp".to_string()],
        "test".to_string(),
    );

    let files: Vec<_> = scanner.scan().collect();
    assert_eq!(files.len(), 1);
    assert!(files[0].path.ends_with("data.txt"));
}

#[test]
fn test_config_defaults() {
    let config = Config::default();
    assert!(config.daemon.socket.to_string_lossy().contains("fan.sock"));
    assert_eq!(config.embedding.model, "all-MiniLM-L6-v2");
    assert_eq!(config.retention.deleted_keep_days, 30);
    assert_eq!(config.schedule.full_sync, "03:00");
    assert!(config.plugins.dir.to_string_lossy().contains("plugins"));
}

#[test]
fn test_sqlite_open_and_status() {
    use fan_core::index::sqlite::SqliteStore;
    let tmp = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(tmp.path()).expect("Failed to open SQLite store");
    let status = store.status().expect("Failed to get status");
    assert_eq!(status.total_files, 0);
    assert_eq!(status.indexed_files, 0);
    assert_eq!(status.deleted_files, 0);
}

#[test]
fn test_sqlite_upsert_and_get() {
    use fan_core::index::sqlite::SqliteStore;
    use fan_core::types::RawFileInfo;
    use std::path::PathBuf;

    let tmp = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(tmp.path()).expect("Failed to open SQLite store");

    let info = RawFileInfo {
        path: PathBuf::from("/test/sample.fastq"),
        source_server: "test".into(),
        size: 1024,
        mtime_secs: 1715299200,
        hash_sha256: None,
        magic_bytes: b"@SEQ_ID".to_vec(),
        mime_type: "text/plain".into(),
    };

    let id = store.upsert(&info, None).expect("Failed to upsert");
    assert!(id > 0);

    let retrieved = store.get_by_id(id).expect("Failed to get by id");
    assert!(retrieved.is_some());
    let entry = retrieved.unwrap();
    assert_eq!(entry.path, PathBuf::from("/test/sample.fastq"));
    assert_eq!(entry.size, 1024);
    assert!(!entry.deleted);
}

#[test]
fn test_sqlite_mark_deleted() {
    use fan_core::index::sqlite::SqliteStore;
    use fan_core::types::RawFileInfo;
    use std::path::PathBuf;

    let tmp = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(tmp.path()).expect("Failed to open SQLite store");

    let info = RawFileInfo {
        path: PathBuf::from("/test/to_delete.fastq"),
        source_server: "test".into(),
        size: 512,
        mtime_secs: 1715299200,
        hash_sha256: None,
        magic_bytes: vec![],
        mime_type: "text/plain".into(),
    };

    store.upsert(&info, None).unwrap();
    store.mark_deleted(&info.path).unwrap();

    let entry = store.get_by_path(&info.path).unwrap().unwrap();
    assert!(entry.deleted);
}

#[test]
fn test_dataset_suggest_queries_are_path_and_species_bounded() {
    use fan_core::index::sqlite::SqliteStore;

    let tmp = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(tmp.path()).unwrap();
    let current_id = store
        .insert_dataset(
            "rice-rna",
            "/data/rice/rna",
            Some("RNA-seq"),
            Some("Oryza sativa"),
            Some("high"),
            None,
        )
        .unwrap();
    store
        .insert_dataset(
            "rice-atac",
            "/data/rice/atac",
            Some("ATAC-seq"),
            Some("Oryza sativa"),
            Some("high"),
            None,
        )
        .unwrap();
    store
        .insert_dataset(
            "maize-atac",
            "/data/maize/atac",
            Some("ATAC-seq"),
            Some("Zea mays"),
            Some("high"),
            None,
        )
        .unwrap();
    store
        .insert_dataset(
            "wildcard-lookalike",
            "/data/riceXrna",
            Some("ATAC-seq"),
            Some("Oryza sativa"),
            Some("high"),
            None,
        )
        .unwrap();

    let current = store
        .find_dataset_for_path("/data/rice/rna/sample")
        .unwrap()
        .unwrap();
    assert_eq!(current.id, current_id);
    assert!(store
        .find_dataset_for_path("/data/rice_rna/sample")
        .unwrap()
        .is_none());

    let candidates = store
        .datasets_by_species("oryza SATIVA", current_id, 10)
        .unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].path, "/data/rice/atac");
}

#[test]
fn test_embedding_lookup_is_restricted_to_candidates() {
    use fan_core::index::sqlite::SqliteStore;
    use fan_core::types::RawFileInfo;
    use std::path::PathBuf;

    let tmp = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(tmp.path()).unwrap();
    let make_file = |path: &str| RawFileInfo {
        path: PathBuf::from(path),
        source_server: "test".into(),
        size: 1,
        mtime_secs: 1,
        hash_sha256: None,
        magic_bytes: Vec::new(),
        mime_type: "application/octet-stream".into(),
    };
    let first = store.upsert(&make_file("/data/first"), None).unwrap();
    let second = store.upsert(&make_file("/data/second"), None).unwrap();
    store.store_embedding(first, &[1.0, 0.0]).unwrap();
    store.store_embedding(second, &[0.0, 1.0]).unwrap();

    let result = store.load_embeddings_for_ids(&[second]).unwrap();
    assert_eq!(result, vec![(second, vec![0.0, 1.0])]);
    assert!(store.load_embeddings_for_ids(&[]).unwrap().is_empty());
}

#[test]
fn test_sqlite_read_only_connection_rejects_writes() {
    use fan_core::index::sqlite::SqliteStore;

    let tmp = tempfile::tempdir().unwrap();
    drop(SqliteStore::open(tmp.path()).unwrap());
    let store = SqliteStore::open_read_only(tmp.path()).unwrap();
    let result = store.insert_dataset(
        "should-fail",
        "/data/should-fail",
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_sqlite_read_only_connection_sees_live_wal_data() {
    use fan_core::index::sqlite::SqliteStore;
    use fan_core::types::RawFileInfo;
    use std::path::PathBuf;

    let tmp = tempfile::tempdir().unwrap();
    let writer = SqliteStore::open(tmp.path()).unwrap();
    let info = RawFileInfo {
        path: PathBuf::from("/data/live.fastq"),
        source_server: "server58".into(),
        size: 128,
        mtime_secs: 1715299200,
        hash_sha256: None,
        magic_bytes: vec![],
        mime_type: "text/plain".into(),
    };
    writer.upsert(&info, None).unwrap();

    // Keep the writer alive so the committed row remains in the live WAL.
    let reader = SqliteStore::open_read_only(tmp.path()).unwrap();
    let entry = reader.get_by_path(&info.path).unwrap().unwrap();
    assert_eq!(entry.size, 128);
    assert_eq!(reader.status().unwrap().total_files, 1);
}
