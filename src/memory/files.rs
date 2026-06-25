use crate::memory::MemoryStore;
use std::error::Error;
use std::path::{Path, PathBuf};

type FileResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[allow(dead_code)]
impl MemoryStore {
    pub fn write_markdown(
        &self,
        relative_path: impl AsRef<Path>,
        content: &str,
    ) -> FileResult<PathBuf> {
        let path = resolve_markdown(self.docs_dir(), relative_path.as_ref())?;
        write_file(&path, content)?;
        Ok(path)
    }

    pub fn append_markdown(
        &self,
        relative_path: impl AsRef<Path>,
        content: &str,
    ) -> FileResult<PathBuf> {
        let path = resolve_markdown(self.docs_dir(), relative_path.as_ref())?;
        append_file(&path, content)?;
        Ok(path)
    }

    pub fn read_markdown(&self, relative_path: impl AsRef<Path>) -> FileResult<String> {
        let path = resolve_markdown(self.docs_dir(), relative_path.as_ref())?;
        Ok(std::fs::read_to_string(path)?)
    }

    pub fn write_skill_file(
        &self,
        relative_path: impl AsRef<Path>,
        content: &str,
    ) -> FileResult<PathBuf> {
        let path = resolve_under(self.skills_dir(), relative_path.as_ref())?;
        write_file(&path, content)?;
        Ok(path)
    }

    pub fn read_skill_file(&self, relative_path: impl AsRef<Path>) -> FileResult<String> {
        let path = resolve_under(self.skills_dir(), relative_path.as_ref())?;
        Ok(std::fs::read_to_string(path)?)
    }
}

fn resolve_markdown(base: &Path, relative_path: &Path) -> FileResult<PathBuf> {
    let raw = relative_path.as_os_str().to_string_lossy();
    if !raw.to_ascii_lowercase().ends_with(".md") {
        return Err("markdown file path must end with .md".into());
    }
    resolve_under(base, relative_path)
}

fn resolve_under(base: &Path, relative_path: &Path) -> FileResult<PathBuf> {
    let raw = relative_path.as_os_str().to_string_lossy();
    let normalized = normalize_relative_path(&raw)?;
    let mut sanitized = PathBuf::new();
    for part in normalized.split('/') {
        sanitized.push(part);
    }

    if sanitized.as_os_str().is_empty() {
        return Err("file path must not be empty".into());
    }

    Ok(base.join(sanitized))
}

fn normalize_relative_path(raw: &str) -> FileResult<String> {
    if raw.is_empty() {
        return Err("file path must not be empty".into());
    }
    if raw.as_bytes().contains(&0) {
        return Err("file path must not contain NUL".into());
    }

    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err("file path must be relative".into());
    }

    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err("file path must not contain parent components".into());
        }
        if has_windows_forbidden_chars(part) {
            return Err("file path contains characters invalid on Windows".into());
        }
        if is_windows_reserved_name(part) {
            return Err("file path contains a Windows reserved name".into());
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return Err("file path must not be empty".into());
    }

    Ok(parts.join("/"))
}

fn has_windows_forbidden_chars(part: &str) -> bool {
    part.contains(':')
        || part
            .chars()
            .any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
}

fn is_windows_reserved_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end_matches(' ')
        .to_ascii_uppercase();

    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=9).contains(&n))
        || stem
            .strip_prefix("LPT")
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=9).contains(&n))
}

fn write_file(path: &Path, content: &str) -> FileResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

fn append_file(path: &Path, content: &str) -> FileResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}
