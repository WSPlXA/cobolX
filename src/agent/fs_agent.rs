use super::AgentRouter;
use super::types::merge_tool_call_deltas;
use super::types::{
    ChatMessage, ChatRequest, FunctionDefinition, StreamOptions, Tool, ToolCall, Usage,
};
use crate::memory::MemoryStore;
use std::path::Path;

fn truncate_utf8_preview(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }

    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

impl AgentRouter {
    /// Validates `user_path` resolves inside `sandbox`.
    /// Returns the canonical absolute path or an error string.
    pub(crate) fn validate_sandbox_path(
        sandbox: &Path,
        user_path: &str,
    ) -> Result<std::path::PathBuf, String> {
        let normalized = if std::path::Path::new(user_path).is_absolute() {
            user_path.to_string()
        } else {
            user_path.trim_start_matches(['/', '\\']).to_string()
        };

        let candidate = if std::path::Path::new(&normalized).is_absolute() {
            std::path::PathBuf::from(&normalized)
        } else {
            sandbox.join(&normalized)
        };

        let sandbox_canon = sandbox
            .canonicalize()
            .map_err(|e| format!("Sandbox path error: {e}"))?;

        let clean_canon = |p: &Path| -> String {
            let s = p.to_string_lossy().into_owned();
            let s_stripped = if let Some(stripped) = s.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                s
            };
            s_stripped.replace('\\', "/").to_lowercase()
        };

        let sandbox_canon_str = clean_canon(&sandbox_canon);

        let mut existing = candidate.clone();
        let mut suffix = std::path::PathBuf::new();
        loop {
            if existing.exists() {
                break;
            }
            if let Some(parent) = existing.parent() {
                if let Some(file_name) = existing.file_name() {
                    suffix = std::path::Path::new(file_name).join(&suffix);
                    existing = parent.to_path_buf();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let canon_existing = existing
            .canonicalize()
            .map_err(|e| format!("Path resolution error: {e}"))?;
        let resolved = if suffix.as_os_str().is_empty() {
            canon_existing.clone()
        } else {
            canon_existing.join(&suffix)
        };
        let resolved_str = clean_canon(&resolved);

        let is_sub = resolved_str == sandbox_canon_str
            || (resolved_str.starts_with(&sandbox_canon_str)
                && (sandbox_canon_str.ends_with('/')
                    || resolved_str.chars().nth(sandbox_canon_str.chars().count()) == Some('/')));

        if !is_sub {
            return Err(format!(
                "Access denied: '{}' is outside the sandbox directory",
                user_path
            ));
        }
        Ok(resolved)
    }

    /// Phase 1 — silent read-only data retrieval (DB + files).
    /// Text from the LLM is captured but NOT sent to `tx`; only STATUS updates are.
    /// Returns the structured data summary once the LLM stops calling tools.
    pub(crate) async fn run_filesystem_retrieval(
        &self,
        initial_messages: &[ChatMessage],
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(String, Option<Usage>), String> {
        let (api_key, api_url, model_name) = if let Some(ref g) = self.glm {
            (
                g.api_key.clone(),
                "https://open.bigmodel.cn/api/paas/v4/chat/completions",
                "glm-4-pro",
            )
        } else if let Some(ref ds) = self.deepseek {
            (
                ds.api_key.clone(),
                "https://api.deepseek.com/chat/completions",
                "deepseek-chat",
            )
        } else {
            return Err("No API client for Filesystem Retrieval.".to_string());
        };

        let http_client = reqwest::Client::new();
        let mut messages = initial_messages.to_vec();
        let sandbox_display = sandbox_path.to_string_lossy();

        if let Some(first_msg) = messages.get_mut(0) {
            if first_msg.role == "system" {
                first_msg.content = Some(format!(
                    "You are the COBOLX Filesystem Retrieval Agent. Your ONLY job is to collect \
                    raw data about COBOL files using the tools below. Do NOT explain or interpret \
                    — just gather and output a structured data summary.\n\
                    \n\
                    Sandbox root: {sandbox_display}\n\
                    Use relative paths for all tool calls (e.g. 'src/MAIN.cbl').\n\
                    \n\
                    WORKFLOW:\n\
                    1. query_sqlite: SELECT id, path, kind FROM files\n\
                    2. query_sqlite: get programs, data_items, call_edges, copybook_uses\n\
                    3. read_file: raw source text only when needed\n\
                    4. list_directory / search_in_file: locate files if needed\n\
                    \n\
                    When done output a STRUCTURED DATA SUMMARY with section headers \
                    (## File, ## Programs, ## Data Items, ## Call Graph, ## COPY Dependencies, \
                    ## Source). Include ALL data. Do not interpret.\n\
                    \n\
                    SQLite Schema:\n\
                    1. files(id, path, kind 'source'|'copybook', size_bytes)\n\
                    2. programs(id, name, file_id)\n\
                    3. copybook_uses(id, from_file_id, copybook_name, resolve_status)\n\
                    4. call_edges(id, caller_program_id, callee_name, kind)\n\
                    5. data_items(id, program_id, name, level, parent_name, pic, usage_clause, section)"
                ));
            }
        }

        let tools = Self::build_readonly_tools();
        let mut final_usage = Usage::default();
        let mut gathered = String::new();

        for _turn in 0..20 {
            let request_body = ChatRequest {
                model: model_name.to_string(),
                messages: messages.clone(),
                stream: true,
                temperature: Some(0.1),
                stream_options: Some(StreamOptions {
                    include_usage: true,
                }),
                tools: Some(tools.clone()),
            };

            let response = http_client
                .post(api_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&request_body)
                .send()
                .await
                .map_err(|e| format!("Network error: {e}"))?;

            if !response.status().is_success() {
                let err_body = response.text().await.unwrap_or_default();
                return Err(format!("Retrieval Agent API error: {err_body}"));
            }

            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut tool_calls_accumulated: Vec<ToolCall> = Vec::new();
            let mut text_this_turn = String::new();

            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res.map_err(|e| format!("Stream read error: {e}"))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].to_string();
                    buffer.drain(..=pos);
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed == "data: [DONE]" {
                        continue;
                    }
                    if let Some(json_str) = trimmed.strip_prefix("data: ") {
                        if let Ok(parsed) =
                            serde_json::from_str::<super::types::ChatResponseStream>(json_str)
                        {
                            if let Some(ref u) = parsed.usage {
                                final_usage.prompt_tokens += u.prompt_tokens;
                                final_usage.completion_tokens += u.completion_tokens;
                                final_usage.total_tokens += u.total_tokens;
                            }
                            if let Some(choice) = parsed.choices.first() {
                                if let Some(ref delta) = choice.delta {
                                    if let Some(ref c) = delta.content {
                                        if !c.is_empty() {
                                            text_this_turn.push_str(c);
                                        }
                                    }
                                    if let Some(ref deltas) = delta.tool_calls {
                                        merge_tool_call_deltas(
                                            &mut tool_calls_accumulated,
                                            deltas.clone(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if tool_calls_accumulated.is_empty() {
                gathered = text_this_turn;
                break;
            }

            let assistant_msg = ChatMessage {
                role: "assistant".to_string(),
                content: if text_this_turn.is_empty() {
                    None
                } else {
                    Some(text_this_turn)
                },
                tool_call_id: None,
                tool_calls: Some(tool_calls_accumulated.clone()),
            };
            messages.push(assistant_msg);

            for tc in &tool_calls_accumulated {
                let result = Self::execute_readonly_tool(tc, sandbox_path, &tx).await?;
                let _ = tx.send("\x01STATUS:".to_string());
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(result),
                    tool_call_id: Some(tc.id.clone()),
                    tool_calls: None,
                });
            }
        }

        let _ = tx.send("\x01STATUS:".to_string());
        Ok((gathered, Some(final_usage)))
    }

    fn build_readonly_tools() -> Vec<Tool> {
        vec![
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "query_sqlite".to_string(),
                    description:
                        "Run one read-only SELECT query against the indexed project SQLite database. \
                        Use this for project facts from files, programs, data_items, call_edges, or \
                        copybook_uses. Do not use it for writes, DDL, or guessed values."
                            .to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "sql": {
                                "type": "string",
                                "description": "A single SQLite SELECT statement that reads indexed project data."
                            }
                        },
                        "required": ["sql"]
                    }),
                },
            },
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "read_file".to_string(),
                    description:
                        "Read the full text of one sandbox file. Use this when exact source content matters, and pass only a path inside the sandbox."
                            .to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Relative path to one file inside the sandbox."
                            }
                        },
                        "required": ["path"]
                    }),
                },
            },
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "list_directory".to_string(),
                    description:
                        "List entries in one sandbox directory, optionally filtered by extension. Use this to discover candidate files before reading them."
                            .to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Relative path to a directory inside the sandbox."
                            },
                            "extension": {
                                "type": "string",
                                "description": "Optional extension filter such as .cbl or .cpy."
                            }
                        },
                        "required": ["path"]
                    }),
                },
            },
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "search_in_file".to_string(),
                    description:
                        "Search one sandbox file for a plain-text pattern, case-insensitive, and return matching lines with line numbers."
                            .to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Relative path to one file inside the sandbox."
                            },
                            "pattern": {
                                "type": "string",
                                "description": "Plain-text pattern to search for."
                            }
                        },
                        "required": ["path", "pattern"]
                    }),
                },
            },
        ]
    }

    async fn execute_readonly_tool(
        tc: &ToolCall,
        sandbox_path: &Path,
        tx: &tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(&tc.function.arguments).unwrap_or_default();
        Ok(match tc.function.name.as_str() {
            "query_sqlite" => {
                let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                let _ = tx.send("\x01STATUS:Querying project database...".to_string());
                match MemoryStore::open_or_create(sandbox_path) {
                    Err(e) => serde_json::json!({ "error": format!("DB error: {e}") }).to_string(),
                    Ok(store) => match store.project_index_is_empty() {
                        Ok(true) => serde_json::json!({
                            "error": "Project index is empty. Run /init before asking for indexed project data."
                        })
                        .to_string(),
                        Ok(false) => match store.query_readonly(sql) {
                            Ok(val) => val.to_string(),
                            Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                        },
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                    },
                }
            }
            "read_file" => {
                let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let _ = tx.send(format!("\x01STATUS:Reading file: {path_str}"));
                match Self::validate_sandbox_path(sandbox_path, path_str) {
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                    Ok(full_path) => match std::fs::read_to_string(&full_path) {
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                        Ok(content) => {
                            const MAX: usize = 120_000;
                            let body = if content.len() > MAX {
                                let preview = truncate_utf8_preview(&content, MAX);
                                format!(
                                    "[truncated: first {MAX} of {} bytes]\n{}",
                                    content.len(),
                                    preview
                                )
                            } else {
                                content
                            };
                            serde_json::json!({ "path": full_path.to_string_lossy(), "content": body })
                                .to_string()
                        }
                    },
                }
            }
            "list_directory" => {
                let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let ext_filter = args.get("extension").and_then(|v| v.as_str());
                let _ = tx.send(format!("\x01STATUS:Listing directory: {path_str}"));
                match Self::validate_sandbox_path(sandbox_path, path_str) {
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                    Ok(full_path) => match std::fs::read_dir(&full_path) {
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                        Ok(entries) => {
                            let sandbox_canon = sandbox_path
                                .canonicalize()
                                .unwrap_or_else(|_| sandbox_path.to_path_buf());
                            let mut files: Vec<serde_json::Value> = entries
                                .filter_map(|e| e.ok())
                                .filter(|e| {
                                    ext_filter.map_or(true, |ext| {
                                        e.path()
                                            .extension()
                                            .and_then(|s| s.to_str())
                                            .map(|s| format!(".{s}").eq_ignore_ascii_case(ext))
                                            .unwrap_or(false)
                                    })
                                })
                                .map(|e| {
                                    let p = e.path();
                                    let rel = p
                                        .strip_prefix(&sandbox_canon)
                                        .unwrap_or(&p)
                                        .to_string_lossy()
                                        .into_owned();
                                    let kind = if p.is_dir() { "dir" } else { "file" };
                                    serde_json::json!({ "name": rel, "kind": kind })
                                })
                                .collect();
                            files.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
                            serde_json::json!({ "entries": files }).to_string()
                        }
                    },
                }
            }
            "search_in_file" => {
                let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                let _ = tx.send(format!("\x01STATUS:Searching '{pattern}' in {path_str}"));
                match Self::validate_sandbox_path(sandbox_path, path_str) {
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                    Ok(full_path) => match std::fs::read_to_string(&full_path) {
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                        Ok(content) => {
                            let pat_lower = pattern.to_lowercase();
                            let matches: Vec<serde_json::Value> = content
                                .lines()
                                .enumerate()
                                .filter(|(_, l)| l.to_lowercase().contains(&pat_lower))
                                .map(|(i, l)| serde_json::json!({ "line": i + 1, "text": l }))
                                .collect();
                            serde_json::json!({
                                "pattern": pattern,
                                "match_count": matches.len(),
                                "matches": matches
                            })
                            .to_string()
                        }
                    },
                }
            }
            unknown => {
                serde_json::json!({ "error": format!("Unknown tool: {unknown}") }).to_string()
            }
        })
    }

    /// Writes a file to the sandbox. If a buffer is provided, it is pushed to the buffer instead of writing physically.
    /// Returns the resolved path or an error string.
    pub(crate) fn write_file(
        &self,
        sandbox: &Path,
        user_path: &str,
        content: &str,
        buffer: Option<&std::sync::Mutex<Vec<(std::path::PathBuf, String)>>>,
    ) -> Result<std::path::PathBuf, String> {
        let full_path = Self::validate_sandbox_path(sandbox, user_path)?;
        if let Some(buf) = buffer {
            if let Ok(mut lock) = buf.lock() {
                lock.push((full_path.clone(), content.to_string()));
            } else {
                return Err("Failed to lock write buffer".to_string());
            }
        } else {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directories: {e}"))?;
            }
            std::fs::write(&full_path, content)
                .map_err(|e| format!("Failed to write file: {e}"))?;
        }
        Ok(full_path)
    }

    /// Commits a list of buffered writes to disk.
    pub(crate) fn commit_write_buffer(
        &self,
        buffer: &[(std::path::PathBuf, String)],
    ) -> Result<(), String> {
        for (full_path, content) in buffer {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directories: {e}"))?;
            }
            std::fs::write(full_path, content).map_err(|e| format!("Failed to write file: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{FunctionCall, ToolCall};
    use std::io::Write;

    #[test]
    fn readonly_tools_descriptions_include_usage_constraints() {
        let tools = AgentRouter::build_readonly_tools();

        let query_sqlite = tools
            .iter()
            .find(|t| t.function.name == "query_sqlite")
            .unwrap();
        assert!(query_sqlite.function.description.contains("SELECT"));
        assert!(query_sqlite.function.description.contains("read-only"));

        let read_file = tools
            .iter()
            .find(|t| t.function.name == "read_file")
            .unwrap();
        assert!(read_file.function.description.contains("sandbox"));
        assert!(read_file.function.description.contains("full text"));

        let list_directory = tools
            .iter()
            .find(|t| t.function.name == "list_directory")
            .unwrap();
        assert!(list_directory.function.description.contains("directory"));
        assert!(list_directory.function.description.contains("extension"));

        let search_in_file = tools
            .iter()
            .find(|t| t.function.name == "search_in_file")
            .unwrap();
        assert!(
            search_in_file
                .function
                .description
                .contains("case-insensitive")
        );
        assert!(search_in_file.function.description.contains("line"));
    }

    #[tokio::test]
    async fn read_file_truncation_handles_utf8_boundaries_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf8.cbl");
        let mut file = std::fs::File::create(&path).unwrap();
        let content = format!("a{}", "你".repeat(40_100));
        file.write_all(content.as_bytes()).unwrap();

        let tc = ToolCall {
            id: "1".to_string(),
            r#type: "function".to_string(),
            function: FunctionCall {
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "utf8.cbl" }).to_string(),
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let result = AgentRouter::execute_readonly_tool(&tc, dir.path(), &tx).await;
        assert!(result.is_ok());

        let result_json = result.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result_json).unwrap();
        let body = parsed["content"].as_str().unwrap_or("");
        assert!(body.contains("[truncated:"), "tool result: {}", result_json);
    }

    #[tokio::test]
    async fn query_sqlite_returns_init_guidance_when_index_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tc = ToolCall {
            id: "2".to_string(),
            r#type: "function".to_string(),
            function: FunctionCall {
                name: "query_sqlite".to_string(),
                arguments: serde_json::json!({ "sql": "SELECT * FROM files" }).to_string(),
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let result = AgentRouter::execute_readonly_tool(&tc, dir.path(), &tx)
            .await
            .unwrap();
        assert!(result.contains("/init"));
        assert!(result.contains("index"));
    }
}
