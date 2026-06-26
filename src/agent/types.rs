use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A buffered sandbox file write (absolute path, content).
pub type WriteBufferEntry = (PathBuf, String);
pub type WriteBuffer = Mutex<Vec<WriteBufferEntry>>;
pub type SharedWriteBuffer = Arc<WriteBuffer>;

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
pub(crate) struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Serialize)]
pub(crate) struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub(crate) struct ChatResponseChoiceMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
pub(crate) struct ChatResponseChoice {
    pub message: ChatResponseChoiceMessage,
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct ToolCallDelta {
    #[serde(default)]
    pub index: usize,
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub function: Option<FunctionCallDelta>,
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ChatResponseStreamChoiceDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize)]
pub(crate) struct ChatResponseStreamChoice {
    #[serde(default)]
    pub delta: Option<ChatResponseStreamChoiceDelta>,
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[allow(dead_code)]
    pub total_tokens: u32,
}

#[derive(Deserialize)]
pub(crate) struct ChatResponseStream {
    #[serde(default)]
    pub choices: Vec<ChatResponseStreamChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Deserialize)]
pub(crate) struct ChatResponse {
    pub choices: Vec<ChatResponseChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Light,
    Heavy,
    Database,
    Filesystem,
}

pub(crate) fn merge_tool_call_deltas(existing: &mut Vec<ToolCall>, deltas: Vec<ToolCallDelta>) {
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
        if let Some(t) = delta.r#type {
            tc.r#type = t;
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
