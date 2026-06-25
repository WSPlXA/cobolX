use rdo::memory::MemoryStore;
use tempfile::tempdir;

#[test]
fn markdown_files_are_created_appended_and_read_under_docs_dir() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::open_or_create(dir.path()).unwrap();

    let path = store
        .write_markdown("analysis/init.md", "# Init\n")
        .unwrap();
    store
        .append_markdown("analysis/init.md", "\nCOBOL inventory ready.\n")
        .unwrap();

    assert!(path.starts_with(store.docs_dir()));
    assert!(path.starts_with(dir.path().join("docs")));
    assert_eq!(
        store.read_markdown("analysis/init.md").unwrap(),
        "# Init\n\nCOBOL inventory ready.\n"
    );
}

#[test]
fn skill_files_are_created_and_read_under_skills_dir() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::open_or_create(dir.path()).unwrap();

    let path = store
        .write_skill_file("cobol-migration/SKILL.md", "# COBOL Migration\n")
        .unwrap();

    assert!(path.starts_with(store.skills_dir()));
    assert_eq!(
        store.read_skill_file("cobol-migration/SKILL.md").unwrap(),
        "# COBOL Migration\n"
    );
}

#[test]
fn windows_style_relative_paths_are_normalized() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::open_or_create(dir.path()).unwrap();

    let path = store
        .write_markdown("analysis\\windows.md", "# Windows\n")
        .unwrap();

    assert!(path.starts_with(dir.path().join("docs")));
    assert_eq!(
        store.read_markdown("analysis/windows.md").unwrap(),
        "# Windows\n"
    );
}

#[test]
fn project_file_writer_rejects_unsafe_or_wrong_paths() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::open_or_create(dir.path()).unwrap();

    assert!(store.write_markdown("../escape.md", "bad").is_err());
    assert!(store.write_markdown("..\\escape.md", "bad").is_err());
    assert!(store.write_markdown("C:\\tmp\\escape.md", "bad").is_err());
    assert!(store.write_markdown("\\\\srv\\share\\x.md", "bad").is_err());
    assert!(store.write_markdown("CON.md", "bad").is_err());
    assert!(store
        .write_markdown("notes/not-markdown.txt", "bad")
        .is_err());
    assert!(store.write_skill_file("../SKILL.md", "bad").is_err());
}
