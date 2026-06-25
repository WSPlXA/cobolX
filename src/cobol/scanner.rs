use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CobolFileType {
    Source,   // .cbl, .cob, .coo
    Copybook, // .cpy
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CobolFileEntry {
    pub path: PathBuf,
    pub file_type: CobolFileType,
    pub size_bytes: u64,
}

/// Recursively scans `dir` for COBOL source and copybook files, ignoring common directories.
/// The root directory is always scanned; exclusion rules only apply to subdirectories.
pub fn scan_sandbox(dir: &Path) -> std::io::Result<Vec<CobolFileEntry>> {
    let mut files = Vec::new();
    scan_dir_entries(dir, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Returns true if a directory name should be excluded from scanning.
fn should_exclude_dir(name: &str) -> bool {
    name.starts_with('.')
        || name == "target"
        || name == "node_modules"
        || name == "vendor"
        || name == "build"
        || name == "tests"
        || name == "test"
}

/// Scans entries within `dir`. Does NOT check exclusion on `dir` itself —
/// callers are responsible for filtering before recursing.
fn scan_dir_entries(dir: &Path, files: &mut Vec<CobolFileEntry>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Apply exclusion only to child directories, not the root
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if should_exclude_dir(name) {
                    continue;
                }
            }
            scan_dir_entries(&path, files)?;
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                let ext_lower = ext.to_lowercase();
                let file_type = match ext_lower.as_str() {
                    "cbl" | "cob" | "coo" => Some(CobolFileType::Source),
                    "cpy" => Some(CobolFileType::Copybook),
                    _ => None,
                };
                if let Some(ft) = file_type {
                    let size_bytes = entry.metadata()?.len();
                    files.push(CobolFileEntry {
                        path,
                        file_type: ft,
                        size_bytes,
                    });
                }
            }
        }
    }
    Ok(())
}
