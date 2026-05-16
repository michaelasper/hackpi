use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBlock {
    Text {
        #[serde(rename = "type")]
        block_type: String,
        text: String,
    },
    ToolUse {
        #[serde(rename = "type")]
        block_type: String,
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        #[serde(rename = "type")]
        block_type: String,
        tool_use_id: String,
        content: String,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text {
            block_type: "text".into(),
            text: text.into(),
        }
    }

    pub fn tool_call(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        ContentBlock::ToolUse {
            block_type: "tool_use".into(),
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            block_type: "tool_result".into(),
            tool_use_id: id.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub endpoint: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8000/v1/messages".into(),
            model: "ds4".into(),
            max_tokens: 8192,
            temperature: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub delta: Option<DeltaPayload>,
    pub content_block: Option<ContentBlockInfo>,
    pub index: Option<u32>,
    pub message: Option<MessageInfo>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaPayload {
    pub text: Option<String>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlockInfo {
    #[serde(rename = "type")]
    pub block_type: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub role: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
