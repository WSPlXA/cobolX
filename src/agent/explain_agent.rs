use super::AgentRouter;
use super::types::merge_tool_call_deltas;
use super::types::{
    ChatMessage, ChatRequest, ChatResponse, FunctionDefinition, StreamOptions, Tool, ToolCall,
    Usage,
};
use std::path::Path;

impl AgentRouter {
    /// Verify agent: reviews a draft answer and returns (passed, feedback).
    async fn run_verify_agent(
        &self,
        user_question: &str,
        gathered_data: &str,
        draft_content: &str,
    ) -> Result<(bool, String), String> {
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
            return Err("No API client for Verify Agent.".to_string());
        };

        let system_prompt = "You are the COBOLX Verify Agent. Review the draft answer.\n\
            Look for: incomplete paragraphs, missing values, TODOs, incorrect analysis, \
            grammar/logic issues, missing files/variables the user asked about.\n\
            Return a JSON object ONLY (no markdown wrapping):\n\
            { \"passed\": bool, \"feedback\": \"...\" }"
            .to_string();

        let user_prompt = format!(
            "User Question:\n{}\n\nGathered Data:\n{}\n\nDraft Answer:\n{}",
            user_question, gathered_data, draft_content
        );

        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: Some(system_prompt),
                tool_call_id: None,
                tool_calls: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: Some(user_prompt),
                tool_call_id: None,
                tool_calls: None,
            },
        ];

        let request_body = ChatRequest {
            model: model_name.to_string(),
            messages,
            stream: false,
            temperature: Some(0.0),
            stream_options: None,
            tools: None,
        };

        let http_client = reqwest::Client::new();
        let response = http_client
            .post(api_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Verify Agent network error: {e}"))?;

        if !response.status().is_success() {
            let err_body = response.text().await.unwrap_or_default();
            return Err(format!("Verify Agent API error: {err_body}"));
        }

        let result: ChatResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Verify Agent response: {}", e))?;

        let raw_content = result
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let json_cleaned = raw_content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        #[derive(serde::Deserialize)]
        struct VerifyResult {
            passed: bool,
            feedback: String,
        }

        let parsed: VerifyResult = serde_json::from_str(json_cleaned)
            .map_err(|e| format!("Verify Agent invalid JSON: {}. Raw: {}", e, raw_content))?;

        Ok((parsed.passed, parsed.feedback))
    }

    /// Runs the explain agent with up to 3 self-correction attempts via the verify agent.
    async fn run_explain_with_verification(
        &self,
        original_messages: &[ChatMessage],
        gathered_data: &str,
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        let mut messages = original_messages.to_vec();
        let mut combined_usage = Usage::default();
        let mut final_model_name = "DeepSeek (Explain Agent)";

        let user_question = original_messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.as_deref())
            .unwrap_or("")
            .to_string();

        for turn in 1..=4 {
            if turn == 4 {
                let _ = tx.send("\x01STATUS:Self-correcting (Final Turn)...".to_string());
                let (usage, model_name) = self
                    .run_explain_agent_stream_internal(
                        &messages,
                        gathered_data,
                        sandbox_path,
                        tx.clone(),
                    )
                    .await?;
                if let Some(u) = usage {
                    combined_usage.prompt_tokens += u.prompt_tokens;
                    combined_usage.completion_tokens += u.completion_tokens;
                    combined_usage.total_tokens += u.total_tokens;
                }
                final_model_name = model_name;
                break;
            }

            let _ = tx.send(format!(
                "\x01STATUS:Explain Agent: Generating draft (Turn {}/3)...",
                turn
            ));

            let (temp_tx, mut temp_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let real_tx_clone = tx.clone();

            let collect_handle = tokio::spawn(async move {
                let mut accumulated_text = String::new();
                let mut accumulated_reasoning = String::new();
                while let Some(msg) = temp_rx.recv().await {
                    if let Some(status) = msg.strip_prefix("\x01STATUS:") {
                        let _ = real_tx_clone.send(format!("\x01STATUS:{}", status));
                    } else if let Some(reasoning) = msg.strip_prefix("\x01REASONING:") {
                        accumulated_reasoning.push_str(reasoning);
                    } else {
                        accumulated_text.push_str(&msg);
                    }
                }
                (accumulated_text, accumulated_reasoning)
            });

            let res = self
                .run_explain_agent_stream_internal(&messages, gathered_data, sandbox_path, temp_tx)
                .await;

            let (accumulated_text, accumulated_reasoning) = match collect_handle.await {
                Ok(tuple) => tuple,
                Err(e) => return Err(format!("Task joining error: {}", e)),
            };

            let (usage, model_name) = res?;
            if let Some(u) = usage {
                combined_usage.prompt_tokens += u.prompt_tokens;
                combined_usage.completion_tokens += u.completion_tokens;
                combined_usage.total_tokens += u.total_tokens;
            }
            final_model_name = model_name;

            let _ = tx.send("\x01STATUS:Verify Agent: Reviewing draft...".to_string());
            match self
                .run_verify_agent(&user_question, gathered_data, &accumulated_text)
                .await
            {
                Ok((passed, feedback)) => {
                    if passed {
                        let _ = tx.send("\x01STATUS:Verify Agent: Validation passed!".to_string());
                        if !accumulated_reasoning.is_empty() {
                            let _ = tx.send(format!("\x01REASONING:{}", accumulated_reasoning));
                        }
                        for chunk in accumulated_text.as_bytes().chunks(64) {
                            if let Ok(s) = std::str::from_utf8(chunk) {
                                let _ = tx.send(s.to_string());
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        }
                        break;
                    } else {
                        let _ = tx.send(
                            "\x01STATUS:Verify Agent: Validation failed, retrying...".to_string(),
                        );
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: Some(accumulated_text),
                            tool_call_id: None,
                            tool_calls: None,
                        });
                        messages.push(ChatMessage {
                            role: "user".to_string(),
                            content: Some(format!(
                                "The previous answer did NOT pass validation:\n{}\n\
                                Please regenerate the full answer and resolve these issues.",
                                feedback
                            )),
                            tool_call_id: None,
                            tool_calls: None,
                        });
                    }
                }
                Err(e) => {
                    let _ = tx.send(format!("\x01STATUS:Verify Agent error: {}, skipping...", e));
                    if !accumulated_reasoning.is_empty() {
                        let _ = tx.send(format!("\x01REASONING:{}", accumulated_reasoning));
                    }
                    for chunk in accumulated_text.as_bytes().chunks(64) {
                        if let Ok(s) = std::str::from_utf8(chunk) {
                            let _ = tx.send(s.to_string());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    break;
                }
            }
        }

        Ok((Some(combined_usage), final_model_name))
    }

    /// Public Phase 2 entry point — delegates to the verification loop.
    pub async fn run_explain_agent_stream(
        &self,
        original_messages: &[ChatMessage],
        gathered_data: &str,
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        self.run_explain_with_verification(original_messages, gathered_data, sandbox_path, tx)
            .await
    }

    /// Internal: single-turn streaming explain agent with optional write_file support.
    async fn run_explain_agent_stream_internal(
        &self,
        original_messages: &[ChatMessage],
        gathered_data: &str,
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        let (api_key, api_url, model_name_static) = if let Some(ref g) = self.glm {
            (
                g.api_key.clone(),
                "https://open.bigmodel.cn/api/paas/v4/chat/completions",
                "GLM-4-Pro (Explain Agent)",
            )
        } else if let Some(ref ds) = self.deepseek {
            (
                ds.api_key.clone(),
                "https://api.deepseek.com/chat/completions",
                "DeepSeek (Explain Agent)",
            )
        } else {
            return Err("No API client for Explain Agent.".to_string());
        };
        let model_id = if self.glm.is_some() {
            "glm-4-pro"
        } else {
            "deepseek-chat"
        };

        let http_client = reqwest::Client::new();

        let user_question = original_messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.as_deref())
            .unwrap_or("")
            .to_string();

        let system_prompt = format!(
            "You are COBOLX, an expert COBOL systems analyst and legacy migration specialist.\n\
            A retrieval agent has already gathered this structured data from the COBOL project:\n\n\
            ---\n{gathered_data}\n---\n\n\
            Using this data, give a thorough, well-structured answer to the user's request.\n\
            - Explanation/analysis: Markdown document covering program purpose, data structures, \
              business logic, CALL graph, COPY dependencies, and migration notes.\n\
            - Documentation (e.g. /docs): use write_file to write Markdown docs to docs/.\n\
            Sandbox root for write_file: {sandbox_display}",
            sandbox_display = sandbox_path.to_string_lossy()
        );

        let mut messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: Some(system_prompt),
                tool_call_id: None,
                tool_calls: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: Some(user_question),
                tool_call_id: None,
                tool_calls: None,
            },
        ];

        let write_file_tool = Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "write_file".to_string(),
                description: "Create or overwrite a file inside the sandbox.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Path relative to sandbox root." },
                        "content": { "type": "string", "description": "Full text content to write." }
                    },
                    "required": ["path", "content"]
                }),
            },
        };

        let mut final_usage = Usage::default();
        let _ = tx.send("\x01STATUS:Explain Agent: Thinking...".to_string());

        for _turn in 0..20 {
            let request_body = ChatRequest {
                model: model_id.to_string(),
                messages: messages.clone(),
                stream: true,
                temperature: Some(0.3),
                stream_options: Some(StreamOptions {
                    include_usage: true,
                }),
                tools: Some(vec![write_file_tool.clone()]),
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
                return Err(format!("Explain Agent API error: {err_body}"));
            }

            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut tool_calls_accumulated: Vec<ToolCall> = Vec::new();
            let mut text_accumulated = String::new();
            let mut status_cleared = false;

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
                                    if let Some(ref reasoning) = delta.reasoning_content {
                                        if !reasoning.is_empty() {
                                            if !status_cleared {
                                                let _ = tx.send("\x01STATUS:".to_string());
                                                status_cleared = true;
                                            }
                                            let _ = tx.send(format!("\x01REASONING:{}", reasoning));
                                        }
                                    }
                                    if let Some(ref c) = delta.content {
                                        if !c.is_empty() {
                                            if !status_cleared {
                                                let _ = tx.send("\x01STATUS:".to_string());
                                                status_cleared = true;
                                            }
                                            text_accumulated.push_str(c);
                                            let _ = tx.send(c.clone());
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

            let assistant_msg = ChatMessage {
                role: "assistant".to_string(),
                content: if text_accumulated.is_empty() {
                    None
                } else {
                    Some(text_accumulated.clone())
                },
                tool_call_id: None,
                tool_calls: Some(tool_calls_accumulated.clone()),
            };
            messages.push(assistant_msg);

            for tc in &tool_calls_accumulated {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                let tool_result = if tc.function.name == "write_file" {
                    let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = tx.send(format!("\x01STATUS:Writing file: {path_str}"));
                    match Self::validate_sandbox_path(sandbox_path, path_str) {
                        Err(e) => serde_json::json!({ "error": e }).to_string(),
                        Ok(full_path) => {
                            if let Some(parent) = full_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            match std::fs::write(&full_path, content) {
                                Ok(_) => serde_json::json!({
                                    "ok": true,
                                    "path": full_path.to_string_lossy()
                                })
                                .to_string(),
                                Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                            }
                        }
                    }
                } else {
                    serde_json::json!({ "error": format!("Unknown tool: {}", tc.function.name) })
                        .to_string()
                };
                let _ = tx.send("\x01STATUS:".to_string());
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(tool_result),
                    tool_call_id: Some(tc.id.clone()),
                    tool_calls: None,
                });
            }
        }

        Ok((Some(final_usage), model_name_static))
    }

    /// Legacy stub — replaced by the two-phase pipeline.
    #[allow(dead_code)]
    async fn run_filesystem_agent_stream(
        &self,
        _initial_messages: &[ChatMessage],
        _sandbox_path: &Path,
        _tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<Option<Usage>, String> {
        Err("Replaced by run_filesystem_retrieval + run_explain_agent_stream".to_string())
    }
}
