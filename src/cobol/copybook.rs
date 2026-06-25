use crate::cobol::scanner::{CobolFileEntry, CobolFileType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) fn build_copybook_index(files: &[CobolFileEntry]) -> HashMap<String, Vec<PathBuf>> {
    let mut index = HashMap::<String, Vec<PathBuf>>::new();
    for file in files {
        if file.file_type != CobolFileType::Copybook {
            continue;
        }
        if let Some(stem) = file.path.file_stem().and_then(|s| s.to_str()) {
            index
                .entry(stem.to_ascii_uppercase())
                .or_default()
                .push(file.path.clone());
        }
    }
    for paths in index.values_mut() {
        paths.sort();
    }
    index
}

pub(crate) fn resolve_copybook(
    root: &Path,
    from_file: &Path,
    name: &str,
    copybook_index: &HashMap<String, Vec<PathBuf>>,
) -> Option<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    if let Some(parent) = from_file.parent() {
        dirs.push(parent.to_path_buf());
    }
    dirs.push(root.to_path_buf());

    for dir in dirs {
        for candidate in candidate_copybook_names(name) {
            let path = dir.join(&candidate);
            if path.is_file() {
                return Some(path);
            }
            if let Some(found) = find_case_insensitive(&dir, &candidate) {
                return Some(found);
            }
        }
    }

    copybook_index
        .get(&name.to_ascii_uppercase())
        .and_then(|paths| paths.first().cloned())
}

fn candidate_copybook_names(name: &str) -> Vec<String> {
    if Path::new(name).extension().is_some() {
        vec![name.to_string()]
    } else {
        vec![name.to_string(), format!("{}.cpy", name)]
    }
}

fn find_case_insensitive(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let target = file_name.to_ascii_uppercase();
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().to_ascii_uppercase() == target {
            return Some(entry.path());
        }
    }
    None
}
