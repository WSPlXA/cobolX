use std::error::Error;
use std::path::{Path, PathBuf};

type MemResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Codex-style: consolidate when cumulative session tokens exceed this.
pub const TOKEN_SUMMARY_THRESHOLD: u32 = 8_000;

/// Codex injects ~5k tokens of `memory_summary.md`; ~4 chars/token.
pub const MEMORY_SUMMARY_INJECT_MAX_CHARS: usize = 20_000;

pub const MEMORY_SUMMARY_FILE: &str = "memory_summary.md";
pub const MEMORY_HANDBOOK_FILE: &str = "MEMORY.md";
pub const RAW_MEMORIES_FILE: &str = "raw_memories.md";
pub const AGENTS_FILE: &str = "AGENTS.md";
pub const LEGACY_SUMMARY_FILE: &str = "SUMMARY_MEM.md";

pub const CONSOLIDATION_SUMMARY_MARKER: &str = "---COBOLX_MEMORY_SUMMARY---";
pub const CONSOLIDATION_HANDBOOK_MARKER: &str = "---COBOLX_MEMORY_HANDBOOK---";

/// Codex-aligned memory layout under `.cobolx/memories/`.
pub struct CodexMemories {
    memories_dir: PathBuf,
    rollout_summaries_dir: PathBuf,
    project_root: PathBuf,
    legacy_summary_path: PathBuf,
}

impl CodexMemories {
    pub fn for_project(base_dir: &Path, project_root: &Path) -> Self {
        let memories_dir = base_dir.join("memories");
        let rollout_summaries_dir = memories_dir.join("rollout_summaries");
        Self {
            memories_dir,
            rollout_summaries_dir,
            project_root: project_root.to_path_buf(),
            legacy_summary_path: base_dir.join(LEGACY_SUMMARY_FILE),
        }
    }

    pub fn memories_dir(&self) -> &Path {
        &self.memories_dir
    }

    pub fn rollout_summaries_dir(&self) -> &Path {
        &self.rollout_summaries_dir
    }

    pub fn ensure_layout(&self) -> MemResult<()> {
        std::fs::create_dir_all(&self.memories_dir)?;
        std::fs::create_dir_all(&self.rollout_summaries_dir)?;

        let summary_path = self.memory_summary_path();
        if !summary_path.exists() {
            if self.legacy_summary_path.exists() {
                let legacy = std::fs::read_to_string(&self.legacy_summary_path)?;
                std::fs::write(&summary_path, legacy)?;
            } else {
                std::fs::write(&summary_path, default_memory_summary())?;
            }
        }

        let handbook_path = self.memory_handbook_path();
        if !handbook_path.exists() {
            std::fs::write(&handbook_path, default_memory_handbook())?;
        }

        Ok(())
    }

    pub fn read_agents_instructions(&self) -> Option<String> {
        let path = self.project_root.join(AGENTS_FILE);
        if path.exists() {
            std::fs::read_to_string(path).ok()
        } else {
            None
        }
    }

    /// Short summary injected each prompt (Codex: `memory_summary.md`, capped).
    pub fn read_memory_summary_for_injection(&self) -> MemResult<String> {
        let raw = std::fs::read_to_string(self.memory_summary_path())?;
        Ok(truncate_utf8_prefix(&raw, MEMORY_SUMMARY_INJECT_MAX_CHARS))
    }

    pub fn read_memory_summary(&self) -> MemResult<String> {
        Ok(std::fs::read_to_string(self.memory_summary_path())?)
    }

    pub fn read_memory_handbook(&self) -> MemResult<String> {
        Ok(std::fs::read_to_string(self.memory_handbook_path())?)
    }

    /// Phase 1: per-run recap (Codex: `rollout_summaries/{id}.md`).
    pub fn write_rollout_summary(&self, run_id: &str, content: &str) -> MemResult<PathBuf> {
        let path = self.rollout_summaries_dir.join(format!("{}.md", run_id));
        std::fs::write(&path, content)?;
        Ok(path)
    }

    pub fn write_raw_memories(&self, content: &str) -> MemResult<()> {
        std::fs::write(self.memories_dir.join(RAW_MEMORIES_FILE), content)?;
        Ok(())
    }

    pub fn write_consolidated(&self, memory_summary: &str, memory_handbook: &str) -> MemResult<()> {
        std::fs::write(self.memory_summary_path(), memory_summary)?;
        std::fs::write(self.memory_handbook_path(), memory_handbook)?;
        Ok(())
    }

    pub fn parse_consolidation_output(output: &str) -> Option<(String, String)> {
        let summary_start = output.find(CONSOLIDATION_SUMMARY_MARKER)?;
        let handbook_start = output.find(CONSOLIDATION_HANDBOOK_MARKER)?;
        if handbook_start <= summary_start {
            return None;
        }
        let summary = output[summary_start + CONSOLIDATION_SUMMARY_MARKER.len()..handbook_start]
            .trim()
            .to_string();
        let handbook = output[handbook_start + CONSOLIDATION_HANDBOOK_MARKER.len()..]
            .trim()
            .to_string();
        if summary.is_empty() || handbook.is_empty() {
            return None;
        }
        Some((summary, handbook))
    }

    fn memory_summary_path(&self) -> PathBuf {
        self.memories_dir.join(MEMORY_SUMMARY_FILE)
    }

    fn memory_handbook_path(&self) -> PathBuf {
        self.memories_dir.join(MEMORY_HANDBOOK_FILE)
    }
}

pub fn default_memory_summary() -> String {
    "# COBOLX Memory Summary\n\n\
     Navigational summary for the next session (Codex-style `memory_summary.md`).\n\n\
     - last_updated: (none)\n\
     - tokens_summarized: 0\n\n\
     ## Context\n\n\
     (none yet)\n\n\
     ## Key findings\n\n\
     (none yet)\n"
        .to_string()
}

pub fn default_memory_handbook() -> String {
    "# COBOLX Memory Handbook\n\n\
     Long-form project memory (Codex-style `MEMORY.md`). You may edit manually.\n\n\
     ## Project\n\n\
     (none yet)\n\n\
     ## COBOL programs\n\n\
     (none yet)\n\n\
     ## Open questions\n\n\
     (none yet)\n"
        .to_string()
}

fn truncate_utf8_prefix(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…\n\n[truncated for context budget]", &content[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn codex_memories_layout_and_rollout_summary() {
        let dir = tempdir().unwrap();
        let base = dir.path().join(".cobolx");
        let mem = CodexMemories::for_project(&base, dir.path());
        mem.ensure_layout().unwrap();

        assert!(mem.memories_dir().join(MEMORY_SUMMARY_FILE).exists());
        assert!(mem.memories_dir().join(MEMORY_HANDBOOK_FILE).exists());

        mem.write_rollout_summary("20250626T120000", "# rollout recap\n")
            .unwrap();
        assert!(
            mem.rollout_summaries_dir()
                .join("20250626T120000.md")
                .exists()
        );
    }

    #[test]
    fn parses_consolidation_markers() {
        let out = format!(
            "preamble\n{}\nsummary body\n{}\nhandbook body",
            CONSOLIDATION_SUMMARY_MARKER, CONSOLIDATION_HANDBOOK_MARKER
        );
        let (s, h) = CodexMemories::parse_consolidation_output(&out).unwrap();
        assert_eq!(s, "summary body");
        assert_eq!(h, "handbook body");
    }
}
