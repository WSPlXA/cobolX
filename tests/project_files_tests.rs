use rdo::path_safety::write_sandbox_file;
use tempfile::tempdir;

#[test]
fn markdown_files_are_created_under_docs_dir() {
    let dir = tempdir().unwrap();
    let sandbox = dir.path();

    let path = write_sandbox_file(sandbox, "docs/analysis/init.md", "# Init\n").unwrap();
    assert!(path.starts_with(sandbox.join("docs")));
    write_sandbox_file(
        sandbox,
        "docs/analysis/init.md",
        "# Init\n\nCOBOL inventory ready.\n",
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.join("docs/analysis/init.md")).unwrap(),
        "# Init\n\nCOBOL inventory ready.\n"
    );
}

#[test]
fn windows_style_relative_paths_are_normalized_for_docs() {
    let dir = tempdir().unwrap();
    let sandbox = dir.path();

    write_sandbox_file(sandbox, "docs\\analysis\\windows.md", "# Windows\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(sandbox.join("docs/analysis/windows.md")).unwrap(),
        "# Windows\n"
    );
}

#[test]
fn sandbox_writer_rejects_unsafe_or_wrong_paths() {
    let dir = tempdir().unwrap();
    let sandbox = dir.path();

    assert!(write_sandbox_file(sandbox, "../escape.md", "bad").is_err());
    assert!(write_sandbox_file(sandbox, "..\\escape.md", "bad").is_err());
    assert!(write_sandbox_file(sandbox, "docs/CON.md", "bad").is_err());
    assert!(write_sandbox_file(sandbox, "docs/notes/not-markdown.txt", "bad").is_err());
}
