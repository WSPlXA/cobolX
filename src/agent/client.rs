use super::clients::{DeepSeekClient, GlmClient};

// Re-exports — tui.rs and other in-crate code uses `use crate::agent::client::{...}`
pub use super::types::{ChatMessage, Route, Usage};
use crate::config::ConfigManager;
use crate::ui::tui::{Message, Sender};
use std::path::Path;

pub struct AgentRouter {
    pub(crate) deepseek: Option<DeepSeekClient>,
    pub(crate) glm: Option<GlmClient>,
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

        let deepseek = env_deepseek.or(file_deepseek).map(DeepSeekClient::new);
        let glm = env_glm.or(file_glm).map(GlmClient::new);

        Self {
            deepseek,
            glm,
            config_path: config_path_str,
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.deepseek.is_some() || self.glm.is_some()
    }

    pub async fn classify_route(&self, prompt: &str) -> Route {
        let system_msg = ChatMessage {
            role: "system".to_string(),
            content: Some(
                "You are the Routing Sub-Agent. Classify the user query into one of:\n\
                - 'LIGHT': simple greetings, basic questions, short chat, definitions.\n\
                - 'HEAVY': programming/coding, algorithms, complex logic, maths, architecture.\n\
                - 'DATABASE': questions about COBOL project structure, file counts, copybook refs, \
                   call graphs, or data variables in the workspace database.\n\
                - 'FILESYSTEM': read/show actual source content of COBOL files; write/generate new \
                   code files; search text patterns inside files; list directory contents; any \
                   migration or refactoring requiring file read/write.\n\
                Output exactly one word: 'LIGHT', 'HEAVY', 'DATABASE', or 'FILESYSTEM'."
                    .to_string(),
            ),
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

        let response = if let Some(ref ds) = self.deepseek {
            ds.call_api(&messages, Some(0.0)).await
        } else if let Some(ref g) = self.glm {
            g.call_api(&messages, Some(0.0)).await
        } else {
            return Route::Light;
        };

        match response {
            Ok(content) => {
                let t = content.trim().to_uppercase();
                if t.contains("FILESYSTEM") {
                    Route::Filesystem
                } else if t.contains("DATABASE") {
                    Route::Database
                } else if t.contains("HEAVY") {
                    Route::Heavy
                } else {
                    Route::Light
                }
            }
            Err(_) => Route::Light,
        }
    }

    fn build_messages(history: &[Message]) -> Vec<ChatMessage> {
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: Some(
                "You are COBOLX, a helpful assistant. COBOLX is a migration agent for legacy \
                COBOL systems based on DeepSeek."
                    .to_string(),
            ),
            tool_call_id: None,
            tool_calls: None,
        }];
        for msg in history {
            let role = match msg.sender {
                Sender::User => "user",
                Sender::Cobolx => "assistant",
            };
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
                role: role.to_string(),
                content: Some(content),
                tool_call_id: None,
                tool_calls: None,
            });
        }
        messages
    }

    #[allow(dead_code)]
    pub async fn execute_chat(
        &self,
        history: &[Message],
        route: Route,
        _sandbox_path: Option<&Path>,
    ) -> Result<(String, &'static str), String> {
        let messages = Self::build_messages(history);
        match route {
            Route::Light => {
                if let Some(ref ds) = self.deepseek {
                    ds.call_api(&messages, None).await.map(|t| (t, "DeepSeek"))
                } else if let Some(ref g) = self.glm {
                    g.call_api(&messages, None)
                        .await
                        .map(|t| (t, "GLM-4-Pro (Fallback)"))
                } else {
                    Err("No API client initialized.".to_string())
                }
            }
            Route::Heavy => {
                if let Some(ref g) = self.glm {
                    g.call_api(&messages, None).await.map(|t| (t, "GLM-4-Pro"))
                } else if let Some(ref ds) = self.deepseek {
                    ds.call_api(&messages, None)
                        .await
                        .map(|t| (t, "DeepSeek (Fallback)"))
                } else {
                    Err("No API client initialized.".to_string())
                }
            }
            Route::Database | Route::Filesystem => {
                Err("This route is only supported in streaming mode.".to_string())
            }
        }
    }

    pub async fn execute_chat_stream(
        &self,
        history: &[Message],
        route: Route,
        sandbox_path: Option<&Path>,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(Option<Usage>, &'static str), String> {
        let messages = Self::build_messages(history);
        match route {
            Route::Light => {
                if let Some(ref ds) = self.deepseek {
                    ds.call_api_stream(&messages, None, tx)
                        .await
                        .map(|u| (u, "DeepSeek"))
                } else if let Some(ref g) = self.glm {
                    g.call_api_stream(&messages, None, tx)
                        .await
                        .map(|u| (u, "GLM-4-Pro (Fallback)"))
                } else {
                    Err("No API client initialized.".to_string())
                }
            }
            Route::Heavy => {
                if let Some(ref g) = self.glm {
                    g.call_api_stream(&messages, None, tx)
                        .await
                        .map(|u| (u, "GLM-4-Pro"))
                } else if let Some(ref ds) = self.deepseek {
                    ds.call_api_stream(&messages, None, tx)
                        .await
                        .map(|u| (u, "DeepSeek (Fallback)"))
                } else {
                    Err("No API client initialized.".to_string())
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
                self.run_database_agent_stream(&messages, path, tx)
                    .await
                    .map(|u| (u, model_name))
            }
            Route::Filesystem => {
                let Some(path) = sandbox_path else {
                    return Err(
                        "Filesystem operations require a configured sandbox path.".to_string()
                    );
                };

                // Phase 1 — silent retrieval
                let _ = tx.send("\x01STATUS:Filesystem: Gathering data...".to_string());
                let (gathered_data, retrieval_usage) = self
                    .run_filesystem_retrieval(&messages, path, tx.clone())
                    .await?;

                // Phase 2 — explain / write
                let (explain_usage, model_name) = self
                    .run_explain_agent_stream(&messages, &gathered_data, path, tx)
                    .await?;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_generation() {
        let router = AgentRouter::new();
        assert!(router.config_path.is_some());
        let path = router.config_path.clone().unwrap();
        let path_buf = std::path::PathBuf::from(path);
        assert!(path_buf.exists());
        let content = std::fs::read_to_string(path_buf).unwrap();
        assert!(content.contains("deepseek_api_key"));
        assert!(content.contains("glm_api_key"));
    }

    #[test]
    fn test_validate_sandbox_path() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = dir.path();

        let res1 = AgentRouter::validate_sandbox_path(sandbox, "docs/README.md");
        assert!(res1.is_ok());

        let abs_path = sandbox.join("src").join("main.cbl");
        let res2 = AgentRouter::validate_sandbox_path(sandbox, &abs_path.to_string_lossy());
        assert!(res2.is_ok());

        let res3 = AgentRouter::validate_sandbox_path(sandbox, "../outside.md");
        assert!(res3.is_err());
    }
}
