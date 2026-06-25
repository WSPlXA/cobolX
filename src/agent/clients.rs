use super::types::{
    ChatMessage, ChatRequest, ChatResponse, ChatResponseStream, StreamOptions, Usage,
};

pub struct DeepSeekClient {
    pub(crate) api_key: String,
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
            buffer.push_str(&String::from_utf8_lossy(&chunk));

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
    pub(crate) api_key: String,
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
            buffer.push_str(&String::from_utf8_lossy(&chunk));

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
