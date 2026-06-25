use crate::ui::tui::Message;
use crate::config::ConfigManager;
use crate::memory::MemoryStore;
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
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize)]
struct ChatResponseStreamChoice {
    delta: ChatResponseStreamChoiceDelta,
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

    pub async fn call_api(&self, messages: &[ChatMessage], temperature: Option<f32>) -> Result<String, String> {
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
            stream_options: Some(StreamOptions { include_usage: true }),
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
                            if let Some(ref content) = choice.delta.content {
                                if !content.is_empty() {
                                    let _ = tx.send(content.clone());
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

    pub async fn call_api(&self, messages: &[ChatMessage], temperature: Option<f32>) -> Result<String, String> {
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
            stream_options: Some(StreamOptions { include_usage: true }),
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
                            if let Some(ref content) = choice.delta.content {
                                if !content.is_empty() {
                                    let _ = tx.send(content.clone());
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

        let file_deepseek = Some(config_data.deepseek_api_key.trim().to_string())
            .filter(|k| !k.is_empty());
        let file_glm = Some(config_data.glm_api_key.trim().to_string())
            .filter(|k| !k.is_empty());

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
    pub async fn execute_chat(&self, history: &[Message], route: Route, _sandbox_path: Option<&Path>) -> Result<(String, &'static str), String> {
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
                    Err("No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY.".to_string())
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
                    Err("No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY.".to_string())
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
                    Err("No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY.".to_string())
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
                    Err("No API client initialized. Set DEEPSEEK_API_KEY or GLM_API_KEY.".to_string())
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
                    return Err("Filesystem operations require a configured sandbox path.".to_string());
                };
                let model_name = if self.glm.is_some() {
                    "GLM-4-Pro (Filesystem Sub-Agent)"
                } else {
                    "DeepSeek (Filesystem Sub-Agent)"
                };
                let res = self.run_filesystem_agent_stream(&messages, path, tx).await;
                res.map(|u| (u, model_name))
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
            (g.api_key.clone(), "https://open.bigmodel.cn/api/paas/v4/chat/completions", "glm-4-pro")
        } else if let Some(ref ds) = self.deepseek {
            (ds.api_key.clone(), "https://api.deepseek.com/chat/completions", "deepseek-chat")
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

        for _turn in 0..5 {
            let request_body = ChatRequest {
                model: model_name.to_string(),
                messages: messages.clone(),
                stream: true,
                temperature: Some(0.0),
                stream_options: Some(StreamOptions { include_usage: true }),
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
                                if let Some(ref content) = choice.delta.content {
                                    if !content.is_empty() {
                                        let _ = tx.send(content.clone());
                                    }
                                }
                                if let Some(ref deltas) = choice.delta.tool_calls {
                                    merge_tool_call_deltas(&mut tool_calls_accumulated, deltas.clone());
                                }
                            }
                        }
                    }
                }
            }

            if !tool_calls_accumulated.is_empty() {
                let _ = tx.send("\x01STATUS:Using Database Sub-Agent: Querying SQLite database...".to_string());

                let assistant_msg = ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_call_id: None,
                    tool_calls: Some(tool_calls_accumulated.clone()),
                };
                messages.push(assistant_msg);

                let store = MemoryStore::open_or_create(sandbox_path)
                    .map_err(|e| format!("Failed to open memory store: {}", e))?;

                for tc in &tool_calls_accumulated {
                    if tc.function.name == "query_sqlite" {
                        let parsed_args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                            .map_err(|e| format!("Failed to parse function arguments: {}", e))?;
                        
                        let sql = parsed_args.get("sql")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let db_result = match store.query_readonly(sql) {
                            Ok(json_val) => json_val.to_string(),
                            Err(err) => serde_json::json!({
                                "error": err.to_string()
                            }).to_string(),
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
    fn validate_sandbox_path(sandbox: &Path, user_path: &str) -> Result<std::path::PathBuf, String> {
        let candidate = if std::path::Path::new(user_path).is_absolute() {
            std::path::PathBuf::from(user_path)
        } else {
            sandbox.join(user_path)
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

    async fn run_filesystem_agent_stream(
        &self,
        initial_messages: &[ChatMessage],
        sandbox_path: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<Option<Usage>, String> {
        let (api_key, api_url, model_name) = if let Some(ref g) = self.glm {
            (g.api_key.clone(), "https://open.bigmodel.cn/api/paas/v4/chat/completions", "glm-4-pro")
        } else if let Some(ref ds) = self.deepseek {
            (ds.api_key.clone(), "https://api.deepseek.com/chat/completions", "deepseek-chat")
        } else {
            return Err("No API client initialized for Filesystem Sub-Agent.".to_string());
        };

        let http_client = reqwest::Client::new();
        let mut messages = initial_messages.to_vec();

        let sandbox_display = sandbox_path.to_string_lossy();

        if let Some(first_msg) = messages.get_mut(0) {
            if first_msg.role == "system" {
                first_msg.content = Some(format!(
                    "You are the COBOLX Filesystem Sub-Agent. You help users read, analyze, and write files \
                    in their COBOL project sandbox.\n\
                    \n\
                    Sandbox root: {sandbox_display}\n\
                    All paths you pass to tools must be relative to the sandbox root (e.g. 'src/MAIN.cbl') \
                    or absolute paths that start with the sandbox root. Absolute paths outside the sandbox \
                    will be rejected.\n\
                    \n\
                    Available tools:\n\
                    - read_file: read the full text content of a file.\n\
                    - write_file: create or overwrite a file with new content.\n\
                    - list_directory: list entries inside a directory, optionally filtered by extension.\n\
                    - search_in_file: search for a text pattern (case-insensitive) and get matching lines with numbers.\n\
                    \n\
                    GUIDELINES:\n\
                    - Always read a file before writing to it if you need to preserve existing content.\n\
                    - When reading large COBOL files, focus on the relevant sections the user asked about.\n\
                    - Prefer relative paths for portability.\n\
                    - If a file does not exist, say so clearly before attempting to write."
                ));
            }
        }

        let read_file_tool = Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "read_file".to_string(),
                description: "Read the full text content of a file inside the sandbox.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path relative to sandbox root, e.g. 'src/PROGRAM.cbl'"
                        }
                    },
                    "required": ["path"]
                }),
            },
        };

        let write_file_tool = Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "write_file".to_string(),
                description: "Create or overwrite a file inside the sandbox with the given content.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path relative to sandbox root, e.g. 'output/Main.java'"
                        },
                        "content": {
                            "type": "string",
                            "description": "The complete text content to write to the file."
                        }
                    },
                    "required": ["path", "content"]
                }),
            },
        };

        let list_directory_tool = Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "list_directory".to_string(),
                description: "List files and subdirectories inside a sandbox directory.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory path relative to sandbox root. Use '.' for the root itself."
                        },
                        "extension": {
                            "type": "string",
                            "description": "Optional extension filter, e.g. '.cbl', '.cpy', '.java'. Omit to list everything."
                        }
                    },
                    "required": ["path"]
                }),
            },
        };

        let search_in_file_tool = Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "search_in_file".to_string(),
                description: "Search for a text pattern (case-insensitive) in a file. Returns matching lines with their line numbers.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path relative to sandbox root."
                        },
                        "pattern": {
                            "type": "string",
                            "description": "Text pattern to search for (plain text, case-insensitive)."
                        }
                    },
                    "required": ["path", "pattern"]
                }),
            },
        };

        let tools = vec![read_file_tool, write_file_tool, list_directory_tool, search_in_file_tool];
        let mut final_usage = Usage::default();

        for _turn in 0..8 {
            let request_body = ChatRequest {
                model: model_name.to_string(),
                messages: messages.clone(),
                stream: true,
                temperature: Some(0.2),
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
                return Err(format!("Filesystem Sub-Agent API error: {err_body}"));
            }

            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut tool_calls_accumulated: Vec<ToolCall> = Vec::new();

            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res.map_err(|e| format!("Stream read error: {e}"))?;
                let chunk_str = String::from_utf8_lossy(&chunk);
                buffer.push_str(&chunk_str);

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].to_string();
                    buffer.drain(..=pos);

                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed == "data: [DONE]" {
                        continue;
                    }
                    if let Some(json_str) = trimmed.strip_prefix("data: ") {
                        if let Ok(parsed) = serde_json::from_str::<ChatResponseStream>(json_str) {
                            if let Some(ref usage) = parsed.usage {
                                final_usage.prompt_tokens += usage.prompt_tokens;
                                final_usage.completion_tokens += usage.completion_tokens;
                                final_usage.total_tokens += usage.total_tokens;
                            }
                            if let Some(choice) = parsed.choices.first() {
                                if let Some(ref content) = choice.delta.content {
                                    if !content.is_empty() {
                                        let _ = tx.send(content.clone());
                                    }
                                }
                                if let Some(ref deltas) = choice.delta.tool_calls {
                                    merge_tool_call_deltas(&mut tool_calls_accumulated, deltas.clone());
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
                content: None,
                tool_call_id: None,
                tool_calls: Some(tool_calls_accumulated.clone()),
            };
            messages.push(assistant_msg);

            for tc in &tool_calls_accumulated {
                let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let tool_result = match tc.function.name.as_str() {
                    "read_file" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = tx.send(format!("\x01STATUS:Reading file: {path_str}"));
                        match Self::validate_sandbox_path(sandbox_path, path_str) {
                            Err(e) => serde_json::json!({ "error": e }).to_string(),
                            Ok(full_path) => match std::fs::read_to_string(&full_path) {
                                Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                                Ok(content) => {
                                    const MAX_BYTES: usize = 120_000;
                                    let truncated = if content.len() > MAX_BYTES {
                                        format!(
                                            "[File truncated: showing first {} of {} bytes]\n{}",
                                            MAX_BYTES,
                                            content.len(),
                                            &content[..MAX_BYTES]
                                        )
                                    } else {
                                        content
                                    };
                                    serde_json::json!({
                                        "path": full_path.to_string_lossy(),
                                        "content": truncated
                                    }).to_string()
                                }
                            },
                        }
                    }

                    "write_file" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = tx.send(format!("\x01STATUS:Writing file: {path_str}"));
                        match Self::validate_sandbox_path(sandbox_path, path_str) {
                            Err(e) => serde_json::json!({ "error": e }).to_string(),
                            Ok(full_path) => {
                                if let Some(parent) = full_path.parent() {
                                    if let Err(e) = std::fs::create_dir_all(parent) {
                                        return Err(format!("Failed to create directories: {e}"));
                                    }
                                }
                                match std::fs::write(&full_path, content) {
                                    Ok(_) => serde_json::json!({
                                        "ok": true,
                                        "path": full_path.to_string_lossy(),
                                        "bytes_written": content.len()
                                    }).to_string(),
                                    Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                                }
                            }
                        }
                    }

                    "list_directory" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                        let ext_filter = args.get("extension").and_then(|v| v.as_str());
                        let _ = tx.send(format!("\x01STATUS:Listing directory: {path_str}"));
                        match Self::validate_sandbox_path(sandbox_path, path_str) {
                            Err(e) => serde_json::json!({ "error": e }).to_string(),
                            Ok(full_path) => {
                                match std::fs::read_dir(&full_path) {
                                    Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                                    Ok(entries) => {
                                        let sandbox_canon = sandbox_path
                                            .canonicalize()
                                            .unwrap_or_else(|_| sandbox_path.to_path_buf());
                                        let mut files: Vec<serde_json::Value> = entries
                                            .filter_map(|e| e.ok())
                                            .filter(|e| {
                                                if let Some(ext) = ext_filter {
                                                    e.path()
                                                        .extension()
                                                        .and_then(|s| s.to_str())
                                                        .map(|s| format!(".{s}").eq_ignore_ascii_case(ext))
                                                        .unwrap_or(false)
                                                } else {
                                                    true
                                                }
                                            })
                                            .map(|e| {
                                                let p = e.path();
                                                let rel = p.strip_prefix(&sandbox_canon)
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
                                }
                            }
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
                                        .filter(|(_, line)| line.to_lowercase().contains(&pat_lower))
                                        .map(|(i, line)| serde_json::json!({
                                            "line": i + 1,
                                            "text": line
                                        }))
                                        .collect();
                                    serde_json::json!({
                                        "pattern": pattern,
                                        "match_count": matches.len(),
                                        "matches": matches
                                    }).to_string()
                                }
                            },
                        }
                    }

                    unknown => serde_json::json!({
                        "error": format!("Unknown tool: {unknown}")
                    }).to_string(),
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

        let _ = tx.send("\x01STATUS:".to_string());
        Ok(Some(final_usage))
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
