use tempfile::tempdir;

#[test]
fn first_open_creates_memory_database_and_schema() {
    let dir = tempdir().unwrap();

    let store = rdo::memory::MemoryStore::open_or_create(dir.path()).unwrap();

    assert!(store.db_path().exists());

    let count: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('files', 'programs', 'runs', 'skills')",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(count, 4);
}
