use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_scan_finds_cobol_files_flat() {
    let dir = tempdir().unwrap();

    // Create mock COBOL files with various supported extensions
    File::create(dir.path().join("main.cbl")).unwrap().write_all(b"IDENTIFICATION DIVISION.").unwrap();
    File::create(dir.path().join("utility.cpy")).unwrap().write_all(b"01 WS-VAR PIC X.").unwrap();
    File::create(dir.path().join("test.cob")).unwrap().write_all(b"PROCEDURE DIVISION.").unwrap();
    File::create(dir.path().join("other.coo")).unwrap().write_all(b"DATA DIVISION.").unwrap();

    // Create non-COBOL files that should be ignored
    File::create(dir.path().join("README.md")).unwrap();
    File::create(dir.path().join("Cargo.toml")).unwrap();

    let result = rdo::cobol::scanner::scan_sandbox(dir.path()).unwrap();

    assert_eq!(result.len(), 4);

    let sources: Vec<_> = result.iter()
        .filter(|f| f.file_type == rdo::cobol::scanner::CobolFileType::Source)
        .collect();
    let copybooks: Vec<_> = result.iter()
        .filter(|f| f.file_type == rdo::cobol::scanner::CobolFileType::Copybook)
        .collect();

    assert_eq!(sources.len(), 3, "Should find 3 source files (.cbl, .cob, .coo)");
    assert_eq!(copybooks.len(), 1, "Should find 1 copybook file (.cpy)");
}

#[test]
fn test_scan_recursive_finds_nested_files() {
    let dir = tempdir().unwrap();

    // Create nested directory structure
    let sub1 = dir.path().join("module_a");
    let sub2 = dir.path().join("module_b");
    let sub2_nested = sub2.join("submodule");
    fs::create_dir_all(&sub1).unwrap();
    fs::create_dir_all(&sub2_nested).unwrap();

    File::create(dir.path().join("main.cbl")).unwrap().write_all(b"ROOT").unwrap();
    File::create(sub1.join("helper.cbl")).unwrap().write_all(b"SUB1").unwrap();
    File::create(sub2.join("process.cob")).unwrap().write_all(b"SUB2").unwrap();
    File::create(sub2_nested.join("deep.cpy")).unwrap().write_all(b"DEEP").unwrap();

    let result = rdo::cobol::scanner::scan_sandbox(dir.path()).unwrap();

    assert_eq!(result.len(), 4, "Should find all 4 COBOL files across nested dirs");

    // Verify paths are sorted
    for i in 1..result.len() {
        assert!(result[i - 1].path <= result[i].path, "Results should be sorted by path");
    }
}

#[test]
fn test_scan_excludes_hidden_and_build_dirs() {
    let dir = tempdir().unwrap();

    // Create directories that should be excluded
    let git_dir = dir.path().join(".git");
    let target_dir = dir.path().join("target");
    let node_modules = dir.path().join("node_modules");
    let vendor_dir = dir.path().join("vendor");
    let build_dir = dir.path().join("build");
    let hidden_dir = dir.path().join(".hidden");
    let valid_dir = dir.path().join("src");

    for d in &[&git_dir, &target_dir, &node_modules, &vendor_dir, &build_dir, &hidden_dir, &valid_dir] {
        fs::create_dir_all(d).unwrap();
    }

    // Put COBOL files in excluded dirs (should NOT be found)
    File::create(git_dir.join("hooks.cbl")).unwrap();
    File::create(target_dir.join("out.cbl")).unwrap();
    File::create(node_modules.join("dep.cbl")).unwrap();
    File::create(vendor_dir.join("lib.cbl")).unwrap();
    File::create(build_dir.join("gen.cbl")).unwrap();
    File::create(hidden_dir.join("secret.cbl")).unwrap();

    // Put COBOL files in valid dirs (SHOULD be found)
    File::create(dir.path().join("root.cbl")).unwrap().write_all(b"ROOT").unwrap();
    File::create(valid_dir.join("app.cob")).unwrap().write_all(b"APP").unwrap();

    let result = rdo::cobol::scanner::scan_sandbox(dir.path()).unwrap();

    assert_eq!(result.len(), 2, "Should only find files outside excluded directories");

    let names: Vec<String> = result.iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"root.cbl".to_string()));
    assert!(names.contains(&"app.cob".to_string()));
}

#[test]
fn test_scan_empty_directory() {
    let dir = tempdir().unwrap();

    let result = rdo::cobol::scanner::scan_sandbox(dir.path()).unwrap();
    assert!(result.is_empty(), "Empty dir should return no results");
}

#[test]
fn test_scan_tracks_file_size() {
    let dir = tempdir().unwrap();

    let content = b"IDENTIFICATION DIVISION.\nPROGRAM-ID. HELLO.\n";
    File::create(dir.path().join("sized.cbl")).unwrap().write_all(content).unwrap();

    let result = rdo::cobol::scanner::scan_sandbox(dir.path()).unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].size_bytes, content.len() as u64);
}
