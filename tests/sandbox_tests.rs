use std::fs::File;
use tempfile::tempdir;

#[test]
fn test_scan_cobol_files() {
    let dir = tempdir().unwrap();
    
    // Create mock COBOL files with various supported extensions
    File::create(dir.path().join("main.cbl")).unwrap();
    File::create(dir.path().join("utility.cpy")).unwrap();
    File::create(dir.path().join("test.cob")).unwrap();
    File::create(dir.path().join("other.coo")).unwrap();
    
    // Create some non-COBOL files that should be ignored
    File::create(dir.path().join("README.md")).unwrap();
    File::create(dir.path().join("Cargo.toml")).unwrap();

    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir.path()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if ["cbl", "cob", "cpy", "coo"].contains(&ext_lower.as_str()) {
                        found.push(path.file_name().unwrap().to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    
    found.sort();
    
    assert_eq!(found.len(), 4);
    assert!(found.contains(&"main.cbl".to_string()));
    assert!(found.contains(&"utility.cpy".to_string()));
    assert!(found.contains(&"test.cob".to_string()));
    assert!(found.contains(&"other.coo".to_string()));
    assert!(!found.contains(&"README.md".to_string()));
}
