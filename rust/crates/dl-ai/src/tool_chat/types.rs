use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default, deserialize_with = "tool_result_content")]
        content: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(deserialize_with = "message_content")]
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolChatRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

impl Usage {
    pub fn total_input(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolChatResponse {
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Option<Usage>,
    pub request_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

fn message_content<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ContentBlock>, D::Error> {
    Ok(match MessageContent::deserialize(deserializer)? {
        MessageContent::Text(text) => vec![ContentBlock::Text { text }],
        MessageContent::Blocks(blocks) => blocks,
    })
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResultBlock {
    Text { text: String },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ResultContent {
    Text(String),
    Blocks(Vec<ResultBlock>),
}

fn tool_result_content<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match ResultContent::deserialize(deserializer)? {
        ResultContent::Text(text) => text,
        ResultContent::Blocks(blocks) => blocks
            .into_iter()
            .map(|ResultBlock::Text { text }| text)
            .collect::<Vec<_>>()
            .join("\n"),
    })
}
