use std::path::{Path, PathBuf};

/// Normalizes a relative path string: rejects `..`, absolute paths, and unsafe components.
pub fn normalize_relative_path(raw: &str) -> Result<String, String> {
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

/// Validates `user_path` resolves inside `sandbox`. Returns the absolute path.
pub fn validate_sandbox_path(sandbox: &Path, user_path: &str) -> Result<PathBuf, String> {
    let normalized = normalize_user_path(user_path)?;
    let sandbox_canon = sandbox
        .canonicalize()
        .map_err(|e| format!("Sandbox path error: {e}"))?;
    let candidate = build_sandbox_candidate(&sandbox_canon, &normalized)?;
    let sandbox_canon_str = clean_canon(&sandbox_canon);
    let candidate_str = clean_canon(&candidate);

    if !is_subpath(&sandbox_canon_str, &candidate_str) {
        return Err(format!(
            "Access denied: '{}' is outside the sandbox directory",
            user_path
        ));
    }
    Ok(candidate)
}

fn normalize_user_path(user_path: &str) -> Result<String, String> {
    if Path::new(user_path).is_absolute() {
        Ok(user_path.replace('\\', "/"))
    } else {
        normalize_relative_path(user_path.trim_start_matches(['/', '\\']))
    }
}

fn build_sandbox_candidate(sandbox_canon: &Path, normalized: &str) -> Result<PathBuf, String> {
    if Path::new(normalized).is_absolute() {
        let candidate = PathBuf::from(normalized);
        let candidate_canon = candidate.canonicalize().unwrap_or(candidate);
        Ok(candidate_canon)
    } else {
        let mut candidate = sandbox_canon.to_path_buf();
        for part in normalized.split('/') {
            if !part.is_empty() {
                candidate.push(part);
            }
        }
        Ok(candidate)
    }
}

/// Extra policy for write operations after sandbox resolution.
pub fn validate_write_path(sandbox: &Path, resolved: &Path) -> Result<(), String> {
    let sandbox_canon = sandbox
        .canonicalize()
        .map_err(|e| format!("Sandbox path error: {e}"))?;
    let sandbox_canon_str = clean_canon(&sandbox_canon);
    let resolved_str = clean_canon(resolved);

    if !is_subpath(&sandbox_canon_str, &resolved_str) {
        return Err("Access denied: path is outside the sandbox directory".into());
    }

    let rel = relative_path_key(&sandbox_canon_str, &resolved_str);

    if is_under_docs(&rel) && !has_markdown_extension(resolved) {
        return Err("paths under docs/ must end with .md".into());
    }

    for part in rel.split('/') {
        if part.is_empty() {
            continue;
        }
        if has_windows_forbidden_chars(part) {
            return Err("file path contains characters invalid on Windows".into());
        }
        if is_windows_reserved_name(part) {
            return Err("file path contains a Windows reserved name".into());
        }
    }

    Ok(())
}

/// Resolves and validates a sandbox write target.
pub fn validate_and_resolve_write(sandbox: &Path, user_path: &str) -> Result<PathBuf, String> {
    let full_path = validate_sandbox_path(sandbox, user_path)?;
    validate_write_path(sandbox, &full_path)?;
    Ok(full_path)
}

/// Writes `content` to an already-validated absolute path.
pub fn write_validated_path(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    std::fs::write(path, content).map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(())
}

/// Validates sandbox write policy and writes `content` to disk.
pub fn write_sandbox_file(
    sandbox: &Path,
    user_path: &str,
    content: &str,
) -> Result<PathBuf, String> {
    let full_path = validate_and_resolve_write(sandbox, user_path)?;
    write_validated_path(&full_path, content)?;
    Ok(full_path)
}

fn clean_canon(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    let s_stripped = if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s
    };
    s_stripped.replace('\\', "/").to_lowercase()
}

fn is_subpath(base: &str, path: &str) -> bool {
    path == base
        || (path.starts_with(base)
            && (base.ends_with('/') || path.chars().nth(base.chars().count()) == Some('/')))
}

fn relative_path_key(sandbox_canon_str: &str, resolved_str: &str) -> String {
    if resolved_str == sandbox_canon_str {
        return String::new();
    }
    let prefix_len = sandbox_canon_str.len();
    if resolved_str.len() > prefix_len && resolved_str.as_bytes().get(prefix_len) == Some(&b'/') {
        resolved_str[prefix_len + 1..].to_string()
    } else {
        String::new()
    }
}

fn is_under_docs(rel: &str) -> bool {
    rel.is_empty() || rel == "docs" || rel.starts_with("docs/")
}

fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_sandbox_path_allows_in_sandbox_and_rejects_escape() {
        let dir = tempdir().unwrap();
        let sandbox = dir.path();

        assert!(validate_sandbox_path(sandbox, "docs/README.md").is_ok());

        let abs_path = sandbox.join("src").join("main.cbl");
        assert!(validate_sandbox_path(sandbox, &abs_path.to_string_lossy()).is_ok());

        assert!(validate_sandbox_path(sandbox, "../outside.md").is_err());
    }

    #[test]
    fn write_sandbox_file_creates_docs_markdown() {
        let dir = tempdir().unwrap();
        let sandbox = dir.path();

        let path = write_sandbox_file(sandbox, "docs/analysis/init.md", "# Init\n").unwrap();
        assert!(path.starts_with(sandbox.join("docs")));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "# Init\n");

        write_sandbox_file(sandbox, "docs/analysis/init.md", "# Init\n\nMore\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(sandbox.join("docs/analysis/init.md")).unwrap(),
            "# Init\n\nMore\n"
        );
    }

    #[test]
    fn write_sandbox_file_allows_non_docs_paths_for_migration() {
        let dir = tempdir().unwrap();
        let sandbox = dir.path();

        let path = write_sandbox_file(sandbox, "src/MAIN.cbl", "IDENTIFICATION DIVISION.").unwrap();
        assert!(path.ends_with("MAIN.cbl"));
    }

    #[test]
    fn windows_style_docs_paths_are_normalized() {
        let dir = tempdir().unwrap();
        let sandbox = dir.path();

        write_sandbox_file(sandbox, "docs\\analysis\\windows.md", "# Windows\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(sandbox.join("docs/analysis/windows.md")).unwrap(),
            "# Windows\n"
        );
    }

    #[test]
    fn write_sandbox_file_rejects_unsafe_or_invalid_paths() {
        let dir = tempdir().unwrap();
        let sandbox = dir.path();

        assert!(write_sandbox_file(sandbox, "../escape.md", "bad").is_err());
        assert!(write_sandbox_file(sandbox, "..\\escape.md", "bad").is_err());
        assert!(write_sandbox_file(sandbox, "docs/CON.md", "bad").is_err());
        assert!(write_sandbox_file(sandbox, "docs/notes/not-markdown.txt", "bad").is_err());
    }

    #[test]
    fn normalize_relative_path_rejects_absolute_and_reserved_names() {
        assert!(normalize_relative_path("C:\\tmp\\escape.md").is_err());
        assert!(normalize_relative_path("\\\\srv\\share\\x.md").is_err());
        assert!(normalize_relative_path("CON.md").is_err());
    }
}
