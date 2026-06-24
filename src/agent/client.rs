use crate::ui::tui::Message;
use crate::config::ConfigManager;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
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
}

#[derive(Deserialize)]
struct ChatResponseChoiceMessage {
    content: String,
}

#[derive(Deserialize)]
struct ChatResponseChoice {
    message: ChatResponseChoiceMessage,
}

#[derive(Deserialize)]
struct ChatResponseStreamChoiceDelta {
    #[serde(default)]
    content: Option<String>,
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
            Ok(choice.message.content.clone())
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
            Ok(choice.message.content.clone())
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
            content: "You are the Routing Sub-Agent. Your task is to analyze the user's query and classify it into one of two categories:\n\
                      - 'LIGHT': simple greetings, basic questions, short chat, definitions.\n\
                      - 'HEAVY': programming/coding questions, algorithm writing, complex logic, mathematics, system architecture, deep analysis.\n\
                      You MUST output exactly one word, either 'LIGHT' or 'HEAVY'. Do not include any punctuation or extra text.".to_string(),
        };
        let user_msg = ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
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
                if trimmed.contains("HEAVY") {
                    Route::Heavy
                } else {
                    Route::Light
                }
            }
            Err(_) => Route::Light, // default to light model on error
        }
    }

    /// Dispatches prompt with dialog history memory to the selected sub-agent
    #[allow(dead_code)]
    pub async fn execute_chat(&self, history: &[Message], route: Route) -> Result<(String, &'static str), String> {
        let mut messages = Vec::new();
        
        // System prompt defining COBOLX identity
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: "You are COBOLX, a helpful assistant. COBOLX is a migration agent for legacy COBOL systems based on DeepSeek.".to_string(),
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
                content,
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
        }
    }

    /// Dispatches prompt with dialog history memory to the selected sub-agent as a stream
    pub async fn execute_chat_stream(
        &self,
        history: &[Message],
        route: Route,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        let mut messages = Vec::new();
        
        // System prompt defining COBOLX identity
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: "You are COBOLX, a helpful assistant. COBOLX is a migration agent for legacy COBOL systems based on DeepSeek.".to_string(),
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
                content,
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
        }
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
