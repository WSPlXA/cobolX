use serde_json::Value;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

type RunResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Append-only session log: `.cobolx/runs/{run_id}/run.md`
pub struct RunJournal {
    run_id: String,
    started_at: String,
    run_dir: PathBuf,
    markdown_path: PathBuf,
    seq: u64,
}

impl RunJournal {
    pub fn start(runs_dir: &Path) -> RunResult<Self> {
        let started_at = chrono::Utc::now().to_rfc3339();
        let run_id = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
        let run_dir = runs_dir.join(&run_id);
        std::fs::create_dir_all(&run_dir)?;

        let markdown_path = run_dir.join("run.md");
        let header = format!(
            "# COBOLX Run {run_id}\n\n\
             - started_at: {started_at}\n\
             - status: running\n\n\
             ---\n"
        );
        std::fs::write(&markdown_path, header)?;

        Ok(Self {
            run_id,
            started_at,
            run_dir,
            markdown_path,
            seq: 0,
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn read_log(&self) -> RunResult<String> {
        Ok(std::fs::read_to_string(&self.markdown_path)?)
    }

    /// Records one operation; failures are ignored so logging never breaks the UI.
    pub fn log(&mut self, kind: &str, payload: Value) {
        self.seq += 1;
        let ts = chrono::Utc::now().to_rfc3339();
        let body = format_payload_md(&payload);
        let section = format!(
            "\n### {kind} — {ts} (seq {seq})\n\n{body}\n",
            kind = kind,
            ts = ts,
            seq = self.seq,
            body = body
        );
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.markdown_path)
        {
            let _ = write!(file, "{}", section);
        }
    }

    pub fn finish(&self, status: &str) {
        let footer = format!(
            "\n---\n\n\
             - finished_at: {}\n\
             - status: {}\n\
             - event_count: {}\n",
            chrono::Utc::now().to_rfc3339(),
            status,
            self.seq
        );
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.markdown_path)
        {
            let _ = write!(file, "{}", footer);
        }
    }
}

fn format_payload_md(payload: &Value) -> String {
    match payload {
        Value::Object(map) if !map.is_empty() => {
            let mut lines = Vec::with_capacity(map.len());
            for (key, value) in map {
                lines.push(format!("- {}: {}", key, format_value_md(value)));
            }
            lines.join("\n")
        }
        other => format_value_md(other),
    }
}

fn format_value_md(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if s.contains('\n') {
                format!("```\n{}\n```", s)
            } else {
                s.clone()
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                "[]".to_string()
            } else {
                items
                    .iter()
                    .map(format_value_md)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        }
        Value::Object(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn run_journal_appends_markdown_sections() {
        let dir = tempdir().unwrap();
        let runs_dir = dir.path().join("runs");
        let mut journal = RunJournal::start(&runs_dir).unwrap();

        journal.log("user_message", json!({ "text": "hello" }));
        journal.log("route_selected", json!({ "route": "DATABASE" }));
        journal.finish("completed");

        let md = std::fs::read_to_string(journal.run_dir().join("run.md")).unwrap();
        assert!(md.contains("# COBOLX Run"));
        assert!(md.contains("### user_message"));
        assert!(md.contains("- text: hello"));
        assert!(md.contains("### route_selected"));
        assert!(md.contains("- route: DATABASE"));
        assert!(md.contains("- status: completed"));
        assert!(md.contains("- event_count: 2"));
    }
}
