use super::AgentRouter;
use super::types::merge_tool_call_deltas;
use super::types::{
    ChatMessage, ChatRequest, FunctionDefinition, StreamOptions, Tool, ToolCall, Usage,
};
use crate::memory::MemoryStore;
use std::path::Path;

fn build_database_query_tool() -> Tool {
    Tool {
        r#type: "function".to_string(),
        function: FunctionDefinition {
            name: "query_sqlite".to_string(),
            description: "Run one read-only SELECT query against the indexed project SQLite database. Use this for project facts from files, programs, data_items, call_edges, copybook_uses, program_features, code_blocks, external_ops, identifiers, literals, copybook_features, and other indexed COBOL metadata. Do not use it for writes, DDL, or guesses."
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
    }
}

impl AgentRouter {
    pub(crate) async fn run_database_agent_stream(
        &self,
        initial_messages: &[ChatMessage],
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<Option<Usage>, String> {
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
            return Err("No API client initialized for Database Sub-Agent.".to_string());
        };

        let http_client = reqwest::Client::new();
        let mut messages = initial_messages.to_vec();

        if let Some(first_msg) = messages.get_mut(0) {
            if first_msg.role == "system" {
                first_msg.content = Some(
                    "You are the COBOLX Database Sub-Agent. Your task is to help the user analyze \
                    their COBOL codebase by querying the local SQLite database. You have access to \
                    the `query_sqlite` tool to execute read-only SELECT queries.\n\
                    Database Schema:\n\
                    1. `files` (id, path, kind 'source'|'copybook', size_bytes, mtime_unix)\n\
                    2. `programs` (id, name, file_id, start_offset, byte_len)\n\
                    3. `copybook_uses` (id, from_file_id, copybook_name, start_offset, byte_len, \
                         resolved_file_id, resolve_status 'resolved'|'missing', replacing_text)\n\
                    4. `call_edges` (id, caller_program_id, callee_name, start_offset, byte_len, \
                         kind 'static'|'dynamic', using_count)\n\
                    5. `data_items` (id, program_id, source_file_id, name, level, parent_name, \
                         pic, usage_clause, occurs, redefines, section, byte_offset, byte_size, \
                         storage_kind, layout_status, start_offset, byte_len)\n\
                    6. `program_features` (program_id, source_file_id, incoming_call_count, \
                         outgoing_call_count, static_call_count, dynamic_call_count, \
                         copybook_use_count, distinct_copybook_count, referenced_by_file_count, \
                         is_entrypoint, has_heavy_copy_usage, data_item_count, paragraph_count, \
                         external_op_count, identifier_count, literal_count)\n\
                    7. `code_blocks` (id, program_id, source_file_id, name, kind \
                         'section'|'paragraph', parent_section, sequence_no, statement_count, \
                         start_offset, byte_len)\n\
                    8. `external_ops` (id, program_id, source_file_id, kind, verb, target, \
                         start_offset, byte_len)\n\
                    9. `identifiers` (id, program_id, source_file_id, kind, value, occurrences, \
                         first_offset)\n\
                    10. `literals` (id, program_id, source_file_id, kind, value, occurrences, \
                         first_offset)\n\
                    11. `copybook_features` (copybook_file_id, copybook_name, \
                         used_by_program_count, used_by_file_count, replacing_use_count, \
                         data_item_count, contains_header_fields, contains_error_fields)\n\n\
                    GUIDELINES:\n\
                    - Write standard SELECT queries only (read-only).\n\
                    - If unsure about columns, query the schema first.\n\
                    - Explain answers clearly; if no data matches, say so."
                        .to_string(),
                );
            }
        }

        let tools = vec![build_database_query_tool()];
        let mut final_usage = Usage::default();

        for _turn in 0..10 {
            let request_body = ChatRequest {
                model: model_name.to_string(),
                messages: messages.clone(),
                stream: true,
                temperature: Some(0.0),
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
                .map_err(|e| format!("Network error: {}", e))?;

            if !response.status().is_success() {
                let err_body = response.text().await.unwrap_or_default();
                return Err(format!("Database Sub-Agent API error: {}", err_body));
            }

            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut tool_calls_accumulated: Vec<ToolCall> = Vec::new();
            let mut text_accumulated = String::new();

            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res.map_err(|e| format!("Stream read error: {}", e))?;
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
                            if let Some(ref usage) = parsed.usage {
                                final_usage.prompt_tokens += usage.prompt_tokens;
                                final_usage.completion_tokens += usage.completion_tokens;
                                final_usage.total_tokens += usage.total_tokens;
                            }
                            if let Some(choice) = parsed.choices.first() {
                                if let Some(ref delta) = choice.delta {
                                    if let Some(ref reasoning) = delta.reasoning_content {
                                        if !reasoning.is_empty() {
                                            let _ = tx.send(format!("\x01REASONING:{}", reasoning));
                                        }
                                    }
                                    if let Some(ref content) = delta.content {
                                        if !content.is_empty() {
                                            text_accumulated.push_str(content);
                                            let _ = tx.send(content.clone());
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
                break;
            }

            let _ = tx.send(
                "\x01STATUS:Using Database Sub-Agent: Querying SQLite database...".to_string(),
            );

            let assistant_msg = ChatMessage {
                role: "assistant".to_string(),
                content: if text_accumulated.is_empty() {
                    None
                } else {
                    Some(text_accumulated)
                },
                tool_call_id: None,
                tool_calls: Some(tool_calls_accumulated.clone()),
            };
            messages.push(assistant_msg);

            let store = MemoryStore::open_or_create(sandbox_path)
                .map_err(|e| format!("Failed to open memory store: {}", e))?;

            for tc in &tool_calls_accumulated {
                if tc.function.name == "query_sqlite" {
                    let parsed_args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .map_err(|e| format!("Failed to parse args: {}", e))?;
                    let sql = parsed_args
                        .get("sql")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let db_result = match store.project_index_is_empty() {
                        Ok(true) => serde_json::json!({
                            "error": "Project index is empty. Run /init before asking database questions."
                        })
                        .to_string(),
                        Ok(false) => match store.query_readonly(sql) {
                            Ok(json_val) => json_val.to_string(),
                            Err(err) => serde_json::json!({ "error": err.to_string() }).to_string(),
                        },
                        Err(err) => serde_json::json!({ "error": err.to_string() }).to_string(),
                    };
                    messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: Some(db_result),
                        tool_call_id: Some(tc.id.clone()),
                        tool_calls: None,
                    });
                }
            }
            let _ = tx.send("\x01STATUS:".to_string());
        }

        let _ = tx.send("\x01STATUS:".to_string());
        Ok(Some(final_usage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_query_tool_description_spells_out_readonly_scope() {
        let tool = build_database_query_tool();
        assert!(tool.function.description.contains("SELECT"));
        assert!(tool.function.description.contains("read-only"));
        assert!(
            tool.function.description.contains(
                "files, programs, data_items, call_edges, copybook_uses, program_features"
            )
        );
    }
}
