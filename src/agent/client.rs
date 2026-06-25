use crate::config::ConfigManager;
use crate::memory::MemoryStore;
use crate::ui::tui::Message;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Tool {
    pub r#type: String,
    pub function: FunctionDefinition,
}

#[derive(Serialize, Clone, Debug)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct ChatResponseChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct ChatResponseChoice {
    message: ChatResponseChoiceMessage,
}

#[derive(Deserialize, Debug, Clone)]
struct ToolCallDelta {
    #[serde(default)]
    index: usize,
    id: Option<String>,
    r#type: Option<String>,
    function: Option<FunctionCallDelta>,
}

#[derive(Deserialize, Debug, Clone)]
struct FunctionCallDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponseStreamChoiceDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize)]
struct ChatResponseStreamChoice {
    #[serde(default)]
    delta: Option<ChatResponseStreamChoiceDelta>,
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[allow(dead_code)]
    pub total_tokens: u32,
}

#[derive(Deserialize)]
struct ChatResponseStream {
    #[serde(default)]
    choices: Vec<ChatResponseStreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatResponseChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Light,
    Heavy,
    Database,
    Filesystem,
}

pub struct DeepSeekClient {
    api_key: String,
    http_client: reqwest::Client,
}

impl DeepSeekClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http_client: reqwest::Client::new(),
        }
    }

    pub async fn call_api(
        &self,
        messages: &[ChatMessage],
        temperature: Option<f32>,
    ) -> Result<String, String> {
        let request_body = ChatRequest {
            model: "deepseek-chat".to_string(),
            messages: messages.to_vec(),
            stream: false,
            temperature,
            stream_options: None,
            tools: None,
        };

        let response = self
            .http_client
            .post("https://api.deepseek.com/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !response.status().is_success() {
            let err_body = response.text().await.unwrap_or_default();
            return Err(format!("DeepSeek API error: {}", err_body));
        }

        let result: ChatResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        if let Some(choice) = result.choices.first() {
            Ok(choice.message.content.clone().unwrap_or_default())
        } else {
            Err("No completion choices returned".to_string())
        }
    }

    pub async fn call_api_stream(
        &self,
        messages: &[ChatMessage],
        temperature: Option<f32>,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<Option<Usage>, String> {
        let request_body = ChatRequest {
            model: "deepseek-chat".to_string(),
            messages: messages.to_vec(),
            stream: true,
            temperature,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            tools: None,
        };

        let response = self
            .http_client
            .post("https://api.deepseek.com/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !response.status().is_success() {
            let err_body = response.text().await.unwrap_or_default();
            return Err(format!("DeepSeek API error: {}", err_body));
        }

        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut final_usage = None;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.map_err(|e| format!("Stream read error: {}", e))?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].to_string();
                buffer.drain(..=pos);

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == "data: [DONE]" {
                    break;
                }
                if let Some(json_str) = trimmed.strip_prefix("data: ") {
                    if let Ok(parsed) = serde_json::from_str::<ChatResponseStream>(json_str) {
                        if let Some(ref usage) = parsed.usage {
                            final_usage = Some(usage.clone());
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
                                        let _ = tx.send(content.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(final_usage)
    }
}

pub struct GlmClient {
    api_key: String,
    http_client: reqwest::Client,
}

impl GlmClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http_client: reqwest::Client::new(),
        }
    }

    pub async fn call_api(
        &self,
        messages: &[ChatMessage],
        temperature: Option<f32>,
    ) -> Result<String, String> {
        let request_body = ChatRequest {
            model: "glm-4-pro".to_string(),
            messages: messages.to_vec(),
            stream: false,
            temperature,
            stream_options: None,
            tools: None,
        };

        let response = self
            .http_client
            .post("https://open.bigmodel.cn/api/paas/v4/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !response.status().is_success() {
            let err_body = response.text().await.unwrap_or_default();
            return Err(format!("GLM-4-Pro API error: {}", err_body));
        }

        let result: ChatResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        if let Some(choice) = result.choices.first() {
            Ok(choice.message.content.clone().unwrap_or_default())
        } else {
            Err("No completion choices returned".to_string())
        }
    }

    pub async fn call_api_stream(
        &self,
        messages: &[ChatMessage],
        temperature: Option<f32>,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<Option<Usage>, String> {
        let request_body = ChatRequest {
            model: "glm-4-pro".to_string(),
            messages: messages.to_vec(),
            stream: true,
            temperature,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            tools: None,
        };

        let response = self
            .http_client
            .post("https://open.bigmodel.cn/api/paas/v4/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !response.status().is_success() {
            let err_body = response.text().await.unwrap_or_default();
            return Err(format!("GLM-4-Pro API error: {}", err_body));
        }

        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut final_usage = None;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.map_err(|e| format!("Stream read error: {}", e))?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].to_string();
                buffer.drain(..=pos);

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == "data: [DONE]" {
                    break;
                }
                if let Some(json_str) = trimmed.strip_prefix("data: ") {
                    if let Ok(parsed) = serde_json::from_str::<ChatResponseStream>(json_str) {
                        if let Some(ref usage) = parsed.usage {
                            final_usage = Some(usage.clone());
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
                                        let _ = tx.send(content.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(final_usage)
    }
}

fn merge_tool_call_deltas(existing: &mut Vec<ToolCall>, deltas: Vec<ToolCallDelta>) {
    for delta in deltas {
        let idx = delta.index;
        while existing.len() <= idx {
            existing.push(ToolCall {
                id: String::new(),
                r#type: "function".to_string(),
                function: FunctionCall {
                    name: String::new(),
                    arguments: String::new(),
                },
            });
        }
        let tc = &mut existing[idx];
        if let Some(id) = delta.id {
            tc.id.push_str(&id);
        }
        if let Some(r#type) = delta.r#type {
            tc.r#type = r#type;
        }
        if let Some(func) = delta.function {
            if let Some(name) = func.name {
                tc.function.name.push_str(&name);
            }
            if let Some(args) = func.arguments {
                tc.function.arguments.push_str(&args);
            }
        }
    }
}

pub struct AgentRouter {
    deepseek: Option<DeepSeekClient>,
    glm: Option<GlmClient>,
    pub config_path: Option<String>,
}

impl AgentRouter {
    pub fn new() -> Self {
        let env_deepseek = std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());

        let env_glm = std::env::var("GLM_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());

        let (config_path_str, config_data) = ConfigManager::load_or_create();

        let file_deepseek =
            Some(config_data.deepseek_api_key.trim().to_string()).filter(|k| !k.is_empty());
        let file_glm = Some(config_data.glm_api_key.trim().to_string()).filter(|k| !k.is_empty());

        let final_deepseek = env_deepseek.or(file_deepseek);
        let final_glm = env_glm.or(file_glm);

        let deepseek = final_deepseek.map(DeepSeekClient::new);
        let glm = final_glm.map(GlmClient::new);

        Self {
            deepseek,
            glm,
            config_path: config_path_str,
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.deepseek.is_some() || self.glm.is_some()
    }

    /// Classifies user input by spawning a Router Sub-Agent
    pub async fn classify_route(&self, prompt: &str) -> Route {
        // Router system instructions
        let system_msg = ChatMessage {
            role: "system".to_string(),
            content: Some("You are the Routing Sub-Agent. Your task is to analyze the user's query and classify it into one of four categories:\n\
                          - 'LIGHT': simple greetings, basic questions, short chat, definitions.\n\
                          - 'HEAVY': programming/coding questions, algorithm writing, complex logic, mathematics, system architecture, deep analysis.\n\
                          - 'DATABASE': questions asking about the COBOL project structure, file counts, copybook references, call graphs, or data variables/layout inside the workspace database.\n\
                          - 'FILESYSTEM': requests to read, open, or show the actual source content of a COBOL file or copybook; requests to write, generate, or create a new code file; requests to search for text patterns inside files; requests to list directory contents; any file migration or refactoring task that requires reading/writing file content directly.\n\
                          You MUST output exactly one word: 'LIGHT', 'HEAVY', 'DATABASE', or 'FILESYSTEM'. Do not include any punctuation or extra text.".to_string()),
            tool_call_id: None,
            tool_calls: None,
        };
        let user_msg = ChatMessage {
            role: "user".to_string(),
            content: Some(prompt.to_string()),
            tool_call_id: None,
            tool_calls: None,
        };
        let messages = vec![system_msg, user_msg];

        // Call the routing model (prefer DeepSeek as it's fast/cheap; fallback to GLM if DeepSeek is missing)
        let response = if let Some(ref ds) = self.deepseek {
            ds.call_api(&messages, Some(0.0)).await // temperature 0 for strict classification
        } else if let Some(ref g) = self.glm {
            g.call_api(&messages, Some(0.0)).await
        } else {
            return Route::Light;
        };

        match response {
            Ok(content) => {
                let trimmed = content.trim().to_uppercase();
                if trimmed.contains("FILESYSTEM") {
                    Route::Filesystem
                } else if trimmed.contains("DATABASE") {
                    Route::Database
                } else if trimmed.contains("HEAVY") {
                    Route::Heavy
                } else {
                    Route::Light
                }
            }
            Err(_) => Route::Light,
        }
    }

    /// Dispatches prompt with dialog history memory to the selected sub-agent
    #[allow(dead_code)]
    pub async fn execute_chat(
        &self,
        history: &[Message],
        route: Route,
        _sandbox_path: Option<&Path>,
    ) -> Result<(String, &'static str), String> {
        let mut messages = Vec::new();

        // System prompt defining COBOLX identity
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some("You are COBOLX, a helpful assistant. COBOLX is a migration agent for legacy COBOL systems based on DeepSeek.".to_string()),
            tool_call_id: None,
            tool_calls: None,
        });

        // Convert TUI local history into model messages (Memory)
        for msg in history {
            let role = match msg.sender {
                crate::ui::tui::Sender::User => "user".to_string(),
                crate::ui::tui::Sender::Cobolx => "assistant".to_string(),
            };
            // Skip mock response text headers or placeholders
            if msg.text.starts_with("Received prompt:")
                || msg.text == "Thinking..."
                || msg.text.starts_with("Routing...")
                || msg.text.starts_with("(Routed:")
            {
                continue;
            }
            let mut content = msg.text.clone();
            if content.starts_with("(Using ") {
                if let Some(idx) = content.find(") ") {
                    content = content[idx + 2..].to_string();
                }
            }
            messages.push(ChatMessage {
                role,
                content: Some(content),
                tool_call_id: None,
                tool_calls: None,
            });
        }

        match route {
            Route::Light => {
                if let Some(ref ds) = self.deepseek {
                    let res = ds.call_api(&messages, None).await;
                    res.map(|text| (text, "DeepSeek"))
                } else if let Some(ref g) = self.glm {
                    let res = g.call_api(&messages, None).await;
                    res.map(|text| (text, "GLM-4-Pro (Fallback)"))
                } else {
                    Err(
                        "No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY."
                            .to_string(),
                    )
                }
            }
            Route::Heavy => {
                if let Some(ref g) = self.glm {
                    let res = g.call_api(&messages, None).await;
                    res.map(|text| (text, "GLM-4-Pro"))
                } else if let Some(ref ds) = self.deepseek {
                    let res = ds.call_api(&messages, None).await;
                    res.map(|text| (text, "DeepSeek (Fallback)"))
                } else {
                    Err(
                        "No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY."
                            .to_string(),
                    )
                }
            }
            Route::Database | Route::Filesystem => {
                Err("This route is only supported in streaming mode.".to_string())
            }
        }
    }

    /// Dispatches prompt with dialog history memory to the selected sub-agent as a stream
    pub async fn execute_chat_stream(
        &self,
        history: &[Message],
        route: Route,
        sandbox_path: Option<&Path>,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        let mut messages = Vec::new();

        // System prompt defining COBOLX identity
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some("You are COBOLX, a helpful assistant. COBOLX is a migration agent for legacy COBOL systems based on DeepSeek.".to_string()),
            tool_call_id: None,
            tool_calls: None,
        });

        // Convert TUI local history into model messages (Memory)
        for msg in history {
            let role = match msg.sender {
                crate::ui::tui::Sender::User => "user".to_string(),
                crate::ui::tui::Sender::Cobolx => "assistant".to_string(),
            };
            // Skip mock response text headers or placeholders
            if msg.text.starts_with("Received prompt:")
                || msg.text == "Thinking..."
                || msg.text.starts_with("Routing...")
                || msg.text.starts_with("(Routed:")
            {
                continue;
            }
            let mut content = msg.text.clone();
            if content.starts_with("(Using ") {
                if let Some(idx) = content.find(") ") {
                    content = content[idx + 2..].to_string();
                }
            }
            messages.push(ChatMessage {
                role,
                content: Some(content),
                tool_call_id: None,
                tool_calls: None,
            });
        }

        match route {
            Route::Light => {
                if let Some(ref ds) = self.deepseek {
                    let res = ds.call_api_stream(&messages, None, tx).await;
                    res.map(|u| (u, "DeepSeek"))
                } else if let Some(ref g) = self.glm {
                    let res = g.call_api_stream(&messages, None, tx).await;
                    res.map(|u| (u, "GLM-4-Pro (Fallback)"))
                } else {
                    Err(
                        "No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY."
                            .to_string(),
                    )
                }
            }
            Route::Heavy => {
                if let Some(ref g) = self.glm {
                    let res = g.call_api_stream(&messages, None, tx).await;
                    res.map(|u| (u, "GLM-4-Pro"))
                } else if let Some(ref ds) = self.deepseek {
                    let res = ds.call_api_stream(&messages, None, tx).await;
                    res.map(|u| (u, "DeepSeek (Fallback)"))
                } else {
                    Err(
                        "No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY."
                            .to_string(),
                    )
                }
            }
            Route::Database => {
                let Some(path) = sandbox_path else {
                    return Err("Database query requires a configured sandbox path.".to_string());
                };
                let model_name = if self.glm.is_some() {
                    "GLM-4-Pro (Database Sub-Agent)"
                } else {
                    "DeepSeek (Database Sub-Agent)"
                };
                let res = self.run_database_agent_stream(&messages, path, tx).await;
                res.map(|u| (u, model_name))
            }
            Route::Filesystem => {
                let Some(path) = sandbox_path else {
                    return Err(
                        "Filesystem operations require a configured sandbox path.".to_string()
                    );
                };

                // Phase 1 — silent data retrieval (DB + file reads, no text to UI)
                let _ = tx.send("\x01STATUS:Filesystem: Gathering data...".to_string());
                let (gathered_data, retrieval_usage) =
                    self.run_filesystem_retrieval(&messages, path, tx.clone()).await?;

                // Phase 2 — stream explanation / write files
                let (explain_usage, model_name) =
                    self.run_explain_agent_stream(&messages, &gathered_data, path, tx).await?;

                // Combine usage from both phases
                let combined = match (retrieval_usage, explain_usage) {
                    (Some(mut r), Some(e)) => {
                        r.prompt_tokens += e.prompt_tokens;
                        r.completion_tokens += e.completion_tokens;
                        r.total_tokens += e.total_tokens;
                        Some(r)
                    }
                    (r, e) => r.or(e),
                };
                Ok((combined, model_name))
            }
        }
    }

    async fn run_database_agent_stream(
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

        // Update system message
        if let Some(first_msg) = messages.get_mut(0) {
            if first_msg.role == "system" {
                first_msg.content = Some("You are the COBOLX Database Sub-Agent. Your task is to help the user analyze their COBOL codebase by querying the local SQLite database. You have access to the `query_sqlite` tool to execute read-only SELECT queries.\n\
                Database Schema:\n\
                1. `files` (id INTEGER PRIMARY KEY, path TEXT, kind TEXT ('source' or 'copybook'), size_bytes INTEGER, mtime_unix INTEGER)\n\
                2. `programs` (id INTEGER PRIMARY KEY, name TEXT, file_id INTEGER, start_offset INTEGER, byte_len INTEGER) - COBOL programs.\n\
                3. `copybook_uses` (id INTEGER PRIMARY KEY, from_file_id INTEGER, copybook_name TEXT, start_offset INTEGER, byte_len INTEGER, resolved_file_id INTEGER, resolve_status TEXT ('resolved', 'missing'), replacing_text TEXT) - COPY book tracking.\n\
                4. `call_edges` (id INTEGER PRIMARY KEY, caller_program_id INTEGER, callee_name TEXT, start_offset INTEGER, byte_len INTEGER, kind TEXT ('static', 'dynamic'), using_count INTEGER) - CALL graphs.\n\
                5. `data_items` (id INTEGER PRIMARY KEY, program_id INTEGER, source_file_id INTEGER, name TEXT, level INTEGER, parent_name TEXT, pic TEXT, usage_clause TEXT, occurs INTEGER, redefines TEXT, section TEXT, byte_offset INTEGER, byte_size INTEGER, storage_kind TEXT, layout_status TEXT, start_offset INTEGER, byte_len INTEGER) - variable details.\n\n\
                GUIDELINES:\n\
                - Write standard SELECT queries to run on SQLite.\n\
                - Make sure the SQL is correct and only executes read-only SELECT statements.\n\
                - If unsure what table columns are, perform queries to check them first.\n\
                - Explain the answers clearly. If no data matches, explain that to the user.".to_string());
            }
        }

        let query_sqlite_tool = Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "query_sqlite".to_string(),
                description: "Run a read-only SELECT query against the local SQLite database indexing the COBOL project structure.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "sql": {
                            "type": "string",
                            "description": "The SQLite SELECT statement to execute."
                        }
                    },
                    "required": ["sql"]
                }),
            },
        };
        let tools = vec![query_sqlite_tool];

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
                let chunk_str = String::from_utf8_lossy(&chunk);
                buffer.push_str(&chunk_str);

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].to_string();
                    buffer.drain(..=pos);

                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "data: [DONE]" {
                        break;
                    }
                    if let Some(json_str) = trimmed.strip_prefix("data: ") {
                        if let Ok(parsed) = serde_json::from_str::<ChatResponseStream>(json_str) {
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

            if !tool_calls_accumulated.is_empty() {
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
                            serde_json::from_str(&tc.function.arguments).map_err(|e| {
                                format!("Failed to parse function arguments: {}", e)
                            })?;

                        let sql = parsed_args
                            .get("sql")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let db_result = match store.query_readonly(sql) {
                            Ok(json_val) => json_val.to_string(),
                            Err(err) => serde_json::json!({
                                "error": err.to_string()
                            })
                            .to_string(),
                        };

                        let tool_msg = ChatMessage {
                            role: "tool".to_string(),
                            content: Some(db_result),
                            tool_call_id: Some(tc.id.clone()),
                            tool_calls: None,
                        };
                        messages.push(tool_msg);
                    }
                }
                let _ = tx.send("\x01STATUS:".to_string());
            } else {
                break;
            }
        }

        let _ = tx.send("\x01STATUS:".to_string());
        Ok(Some(final_usage))
    }

    /// Validates that `user_path` resolves to a location inside `sandbox`.
    /// Returns the canonical absolute path if safe, or an error string.
    fn validate_sandbox_path(
        sandbox: &Path,
        user_path: &str,
    ) -> Result<std::path::PathBuf, String> {
        // Strip leading separators from non-absolute paths so that an LLM-generated
        // path like "/docs/README.md" is treated as "docs/README.md" relative to the
        // sandbox root, rather than escaping to the drive root on Windows.
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

        // Resolve the sandbox root first so the comparison is reliable even if the
        // sandbox path itself contains symlinks.
        let sandbox_canon = sandbox
            .canonicalize()
            .map_err(|e| format!("Sandbox path error: {e}"))?;

        // The target may not exist yet (e.g. write_file creating a new file).
        // Walk up to the first existing ancestor, canonicalize that, then re-attach
        // the remaining suffix so we can check containment without requiring the
        // leaf to exist.
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
        let resolved = canon_existing.join(&suffix);

        if !resolved.starts_with(&sandbox_canon) {
            return Err(format!(
                "Access denied: '{}' is outside the sandbox directory",
                user_path
            ));
        }
        Ok(resolved)
    }

    /// Phase 1 — read-only data retrieval.
    /// Queries the DB and reads files silently (only STATUS updates go to `tx`).
    /// Returns the structured data summary the LLM produced after tool use.
    async fn run_filesystem_retrieval(
        &self,
        initial_messages: &[ChatMessage],
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(String, Option<Usage>), String> {
        let (api_key, api_url, model_name) = if let Some(ref g) = self.glm {
            (g.api_key.clone(), "https://open.bigmodel.cn/api/paas/v4/chat/completions", "glm-4-pro")
        } else if let Some(ref ds) = self.deepseek {
            (ds.api_key.clone(), "https://api.deepseek.com/chat/completions", "deepseek-chat")
        } else {
            return Err("No API client for Filesystem Retrieval.".to_string());
        };

        let http_client = reqwest::Client::new();
        let mut messages = initial_messages.to_vec();
        let sandbox_display = sandbox_path.to_string_lossy();

        if let Some(first_msg) = messages.get_mut(0) {
            if first_msg.role == "system" {
                first_msg.content = Some(format!(
                    "You are the COBOLX Filesystem Retrieval Agent. Your ONLY job is to \
                    collect raw data about COBOL files using the tools below. \
                    Do NOT explain or interpret — just gather and output a structured data summary.\n\
                    \n\
                    Sandbox root: {sandbox_display}\n\
                    Use relative paths for all tool calls (e.g. 'src/MAIN.cbl').\n\
                    \n\
                    WORKFLOW:\n\
                    1. query_sqlite: get file ids and paths — SELECT id, path, kind FROM files\n\
                    2. query_sqlite: get programs, data_items, call_edges, copybook_uses for each file\n\
                    3. read_file: read source text only when the raw code is needed\n\
                    4. list_directory / search_in_file: use if needed to locate files\n\
                    \n\
                    When done, output a STRUCTURED DATA SUMMARY with clear section headers \
                    (## File, ## Programs, ## Data Items, ## Call Graph, ## COPY Dependencies, ## Source). \
                    Include ALL data found. Do not interpret — another agent will do that.\n\
                    \n\
                    SQLite Schema:\n\
                    1. files(id, path, kind 'source'|'copybook', size_bytes)\n\
                    2. programs(id, name, file_id)\n\
                    3. copybook_uses(id, from_file_id, copybook_name, resolve_status)\n\
                    4. call_edges(id, caller_program_id, callee_name, kind)\n\
                    5. data_items(id, program_id, source_file_id, name, level, parent_name, pic, usage_clause, section)"
                ));
            }
        }

        // Read-only tools only — no write_file
        let tools = Self::build_readonly_tools();
        let mut final_usage = Usage::default();
        let mut gathered = String::new();

        for _turn in 0..20 {
            let request_body = ChatRequest {
                model: model_name.to_string(),
                messages: messages.clone(),
                stream: true,
                temperature: Some(0.1),
                stream_options: Some(StreamOptions { include_usage: true }),
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
                    if trimmed.is_empty() || trimmed == "data: [DONE]" { continue; }
                    if let Some(json_str) = trimmed.strip_prefix("data: ") {
                        if let Ok(parsed) = serde_json::from_str::<ChatResponseStream>(json_str) {
                            if let Some(ref u) = parsed.usage {
                                final_usage.prompt_tokens += u.prompt_tokens;
                                final_usage.completion_tokens += u.completion_tokens;
                                final_usage.total_tokens += u.total_tokens;
                            }
                            if let Some(choice) = parsed.choices.first() {
                                if let Some(ref delta) = choice.delta {
                                    if let Some(ref c) = delta.content {
                                        if !c.is_empty() { text_this_turn.push_str(c); }
                                    }
                                    if let Some(ref deltas) = delta.tool_calls {
                                        merge_tool_call_deltas(&mut tool_calls_accumulated, deltas.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if tool_calls_accumulated.is_empty() {
                // LLM produced final summary — capture it
                gathered = text_this_turn;
                break;
            }

            let assistant_msg = ChatMessage {
                role: "assistant".to_string(),
                content: if text_this_turn.is_empty() { None } else { Some(text_this_turn) },
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

    /// Phase 2 — synthesise, explain, and optionally write files.
    /// Receives the structured data summary from Phase 1 as context.
    async fn run_explain_agent_stream(
        &self,
        original_messages: &[ChatMessage],
        gathered_data: &str,
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        let (api_key, api_url, model_name_static) = if let Some(ref g) = self.glm {
            (g.api_key.clone(), "https://open.bigmodel.cn/api/paas/v4/chat/completions", "GLM-4-Pro (Explain Agent)")
        } else if let Some(ref ds) = self.deepseek {
            (ds.api_key.clone(), "https://api.deepseek.com/chat/completions", "DeepSeek (Explain Agent)")
        } else {
            return Err("No API client for Explain Agent.".to_string());
        };
        let model_id = if self.glm.is_some() { "glm-4-pro" } else { "deepseek-chat" };

        let http_client = reqwest::Client::new();

        // Extract the original user question (last user turn)
        let user_question = original_messages.iter().rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.as_deref())
            .unwrap_or("")
            .to_string();

        let system_prompt = format!(
            "You are COBOLX, an expert COBOL systems analyst and legacy migration specialist.\n\
            A dedicated retrieval agent has already gathered the following structured data from \
            the COBOL project:\n\n\
            ---\n\
            {gathered_data}\n\
            ---\n\n\
            Using this data, give a thorough, well-structured answer to the user's request.\n\
            - If the task is explanation/analysis: produce a clear Markdown document covering \
              program purpose, data structures, business logic, CALL graph, COPY dependencies, \
              and migration notes.\n\
            - If the task is to generate documentation files (e.g. /docs): use write_file to \
              write the Markdown documents to the docs/ directory.\n\
            Sandbox root for write_file paths: {sandbox_display}",
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

        // Give the explain agent only write_file (no read tools — data is already in context)
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
                stream_options: Some(StreamOptions { include_usage: true }),
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
                    if trimmed.is_empty() || trimmed == "data: [DONE]" { continue; }
                    if let Some(json_str) = trimmed.strip_prefix("data: ") {
                        if let Ok(parsed) = serde_json::from_str::<ChatResponseStream>(json_str) {
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
                                            // Clear "Explain Agent: Thinking..." on first real token
                                            if !status_cleared {
                                                let _ = tx.send("\x01STATUS:".to_string());
                                                status_cleared = true;
                                            }
                                            text_accumulated.push_str(c);
                                            let _ = tx.send(c.clone());
                                        }
                                    }
                                    if let Some(ref deltas) = delta.tool_calls {
                                        merge_tool_call_deltas(&mut tool_calls_accumulated, deltas.clone());
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
                content: if text_accumulated.is_empty() { None } else { Some(text_accumulated.clone()) },
                tool_call_id: None,
                tool_calls: Some(tool_calls_accumulated.clone()),
            };
            messages.push(assistant_msg);

            for tc in &tool_calls_accumulated {
                let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or_default();
                let tool_result = if tc.function.name == "write_file" {
                    let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let content  = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = tx.send(format!("\x01STATUS:Writing file: {path_str}"));
                    match Self::validate_sandbox_path(sandbox_path, path_str) {
                        Err(e) => serde_json::json!({ "error": e }).to_string(),
                        Ok(full_path) => {
                            if let Some(parent) = full_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            match std::fs::write(&full_path, content) {
                                Ok(_) => serde_json::json!({ "ok": true, "path": full_path.to_string_lossy() }).to_string(),
                                Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                            }
                        }
                    }
                } else {
                    serde_json::json!({ "error": format!("Unknown tool: {}", tc.function.name) }).to_string()
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

    /// Read-only tool definitions shared by the retrieval agent.
    fn build_readonly_tools() -> Vec<Tool> {
        vec![
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "query_sqlite".to_string(),
                    description: "Run a read-only SELECT query against the project SQLite database (files, programs, data_items, call_edges, copybook_uses).".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": { "sql": { "type": "string" } },
                        "required": ["sql"]
                    }),
                },
            },
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "read_file".to_string(),
                    description: "Read the full text of a sandbox file.".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }),
                },
            },
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "list_directory".to_string(),
                    description: "List entries in a sandbox directory.".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "extension": { "type": "string" }
                        },
                        "required": ["path"]
                    }),
                },
            },
            Tool {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "search_in_file".to_string(),
                    description: "Search for a text pattern (case-insensitive) in a file.".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "pattern": { "type": "string" }
                        },
                        "required": ["path", "pattern"]
                    }),
                },
            },
        ]
    }

    /// Execute a read-only tool call, sending STATUS updates through `tx`.
    async fn execute_readonly_tool(
        tc: &ToolCall,
        sandbox_path: &Path,
        tx: &tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<String, String> {
        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
            .unwrap_or_default();
        Ok(match tc.function.name.as_str() {
            "query_sqlite" => {
                let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                let _ = tx.send("\x01STATUS:Querying project database...".to_string());
                match MemoryStore::open_or_create(sandbox_path) {
                    Err(e) => serde_json::json!({ "error": format!("DB error: {e}") }).to_string(),
                    Ok(store) => match store.query_readonly(sql) {
                        Ok(val) => val.to_string(),
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
                                format!("[truncated: first {MAX} of {} bytes]\n{}", content.len(), &content[..MAX])
                            } else {
                                content
                            };
                            serde_json::json!({ "path": full_path.to_string_lossy(), "content": body }).to_string()
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
                            let sandbox_canon = sandbox_path.canonicalize()
                                .unwrap_or_else(|_| sandbox_path.to_path_buf());
                            let mut files: Vec<serde_json::Value> = entries
                                .filter_map(|e| e.ok())
                                .filter(|e| {
                                    ext_filter.map_or(true, |ext| {
                                        e.path().extension().and_then(|s| s.to_str())
                                            .map(|s| format!(".{s}").eq_ignore_ascii_case(ext))
                                            .unwrap_or(false)
                                    })
                                })
                                .map(|e| {
                                    let p = e.path();
                                    let rel = p.strip_prefix(&sandbox_canon)
                                        .unwrap_or(&p).to_string_lossy().into_owned();
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
                let pattern  = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                let _ = tx.send(format!("\x01STATUS:Searching '{pattern}' in {path_str}"));
                match Self::validate_sandbox_path(sandbox_path, path_str) {
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                    Ok(full_path) => match std::fs::read_to_string(&full_path) {
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                        Ok(content) => {
                            let pat_lower = pattern.to_lowercase();
                            let matches: Vec<serde_json::Value> = content.lines().enumerate()
                                .filter(|(_, l)| l.to_lowercase().contains(&pat_lower))
                                .map(|(i, l)| serde_json::json!({ "line": i + 1, "text": l }))
                                .collect();
                            serde_json::json!({ "pattern": pattern, "match_count": matches.len(), "matches": matches }).to_string()
                        }
                    },
                }
            }
            unknown => serde_json::json!({ "error": format!("Unknown tool: {unknown}") }).to_string(),
        })
    }

    // Legacy single-phase filesystem agent kept for compatibility.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_generation() {
        let router = AgentRouter::new();
        assert!(router.config_path.is_some());
        let path = router.config_path.clone().unwrap();
        println!("Generated config path: {}", path);
        let path_buf = std::path::PathBuf::from(path);
        assert!(path_buf.exists());
        let content = std::fs::read_to_string(path_buf).unwrap();
        assert!(content.contains("deepseek_api_key"));
        assert!(content.contains("glm_api_key"));
    }
}
