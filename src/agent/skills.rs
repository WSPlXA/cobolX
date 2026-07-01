use std::path::{Path, PathBuf};

pub(crate) const MAX_AGENT_SKILL_CHARS: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentKind {
    Database,
    FilesystemRetrieval,
    Explain,
    Verify,
}

impl AgentKind {
    pub(crate) fn skill_dir(self) -> &'static str {
        match self {
            AgentKind::Database => "database",
            AgentKind::FilesystemRetrieval => "filesystem",
            AgentKind::Explain => "explain",
            AgentKind::Verify => "verify",
        }
    }

    fn prompt_label(self) -> &'static str {
        match self {
            AgentKind::Database => "Database Sub-Agent",
            AgentKind::FilesystemRetrieval => "Filesystem Retrieval Agent",
            AgentKind::Explain => "Explain Agent",
            AgentKind::Verify => "Verify Agent",
        }
    }
}

pub(crate) const AGENT_SKILL_DIRS: &[&str] = &["database", "filesystem", "explain", "verify"];

pub(crate) fn append_agent_skills(
    system_prompt: &mut String,
    project_root: &Path,
    agent: AgentKind,
) -> Result<(), String> {
    if let Some(skills) = load_agent_skills(project_root, agent)? {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&skills);
    }
    Ok(())
}

pub(crate) fn load_agent_skills(
    project_root: &Path,
    agent: AgentKind,
) -> Result<Option<String>, String> {
    let skill_root = project_root
        .join(".cobolx")
        .join("skills")
        .join(agent.skill_dir());
    if !skill_root.exists() {
        return Ok(None);
    }
    if !skill_root.is_dir() {
        return Err(format!(
            "Agent skill path is not a directory: {}",
            skill_root.to_string_lossy()
        ));
    }

    let mut files = Vec::new();
    collect_skill_files(&skill_root, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Ok(None);
    }

    let mut out = format!(
        "## Agent Skills ({})\n\
         These instructions are scoped to this sub-agent only. Do not infer that other agents saw them.\n\n",
        agent.prompt_label()
    );

    for path in files {
        let rel = path
            .strip_prefix(project_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let header = format!("### {}\n\n", rel);
        if !push_bounded(&mut out, &header, MAX_AGENT_SKILL_CHARS) {
            return Ok(Some(out));
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read skill {}: {e}", path.to_string_lossy()))?;
        if !push_bounded(&mut out, content.trim(), MAX_AGENT_SKILL_CHARS) {
            return Ok(Some(out));
        }
        if !push_bounded(&mut out, "\n\n", MAX_AGENT_SKILL_CHARS) {
            return Ok(Some(out));
        }
    }

    Ok(Some(out))
}

fn collect_skill_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read skill directory {}: {e}", dir.to_string_lossy()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read skill directory entry: {e}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to read file type {}: {e}", path.to_string_lossy()))?;

        if file_type.is_dir() {
            if is_hidden_path(&path) {
                continue;
            }
            collect_skill_files(&path, files)?;
        } else if file_type.is_file() && is_markdown_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn is_hidden_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn push_bounded(out: &mut String, text: &str, max_bytes: usize) -> bool {
    if out.len() >= max_bytes {
        return false;
    }
    let remaining = max_bytes - out.len();
    if text.len() <= remaining {
        out.push_str(text);
        return true;
    }

    const MARKER: &str = "\n\n[agent skills truncated]\n";
    let budget = remaining.saturating_sub(MARKER.len());
    let mut end = budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&text[..end]);
    if out.len() + MARKER.len() <= max_bytes {
        out.push_str(MARKER);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_only_the_requested_agent_skill_directory() {
        let dir = tempdir().unwrap();
        let database_dir = dir.path().join(".cobolx/skills/database");
        let explain_dir = dir.path().join(".cobolx/skills/explain");
        std::fs::create_dir_all(&database_dir).unwrap();
        std::fs::create_dir_all(&explain_dir).unwrap();
        std::fs::write(database_dir.join("query.md"), "DB_ONLY").unwrap();
        std::fs::write(explain_dir.join("report.md"), "EXPLAIN_ONLY").unwrap();

        let loaded = load_agent_skills(dir.path(), AgentKind::Database)
            .unwrap()
            .unwrap();

        assert!(loaded.contains("DB_ONLY"));
        assert!(!loaded.contains("EXPLAIN_ONLY"));
    }

    #[test]
    fn loads_markdown_skills_in_deterministic_order() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join(".cobolx/skills/filesystem/nested");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.parent().unwrap().join("b.md"), "B").unwrap();
        std::fs::write(skill_dir.parent().unwrap().join("a.md"), "A").unwrap();
        std::fs::write(skill_dir.join("c.txt"), "ignored").unwrap();

        let loaded = load_agent_skills(dir.path(), AgentKind::FilesystemRetrieval)
            .unwrap()
            .unwrap();
        let a = loaded.find("a.md").unwrap();
        let b = loaded.find("b.md").unwrap();

        assert!(a < b);
        assert!(!loaded.contains("ignored"));
    }

    #[test]
    fn bounds_skill_injection_at_utf8_boundary() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join(".cobolx/skills/verify");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("large.md"), "你".repeat(10_000)).unwrap();

        let loaded = load_agent_skills(dir.path(), AgentKind::Verify)
            .unwrap()
            .unwrap();

        assert!(loaded.len() <= MAX_AGENT_SKILL_CHARS);
        assert!(loaded.is_char_boundary(loaded.len()));
        assert!(loaded.contains("agent skills truncated"));
    }
}
