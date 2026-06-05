//! Core types for llmrust — unified LLM API interface.

use serde::{Deserialize, Serialize};

/// Chat message role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// A tool/function result fed back into the conversation.
    Tool,
}

/// An OpenAI-style tool the model may call. Currently only `function` tools
/// are supported.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

/// The schema of a callable function advertised to the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the function's parameters.
    pub parameters: serde_json::Value,
}

impl Tool {
    /// Build a `function` tool from a name, optional description, and a JSON
    /// Schema for its parameters.
    pub fn function(
        name: impl Into<String>,
        description: Option<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: name.into(),
                description,
                parameters,
            },
        }
    }
}

/// Controls whether (and which) tool the model may call.
///
/// Serializes as either a bare string (`"auto"`, `"none"`, `"required"`) or an
/// object selecting a specific function, matching the OpenAI wire format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ToolChoice {
    /// One of `"auto"`, `"none"`, or `"required"`.
    Mode(String),
    /// Force a specific function call.
    Function {
        #[serde(rename = "type")]
        choice_type: String,
        function: ToolChoiceFunction,
    },
}

/// The function selected by a [`ToolChoice::Function`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolChoiceFunction {
    pub name: String,
}

impl ToolChoice {
    /// Let the model decide whether to call a tool (`"auto"`).
    pub fn auto() -> Self {
        Self::Mode("auto".to_string())
    }

    /// Forbid tool calls (`"none"`).
    pub fn none() -> Self {
        Self::Mode("none".to_string())
    }

    /// Require the model to call at least one tool (`"required"`).
    pub fn required() -> Self {
        Self::Mode("required".to_string())
    }

    /// Force the model to call a specific function by name.
    pub fn function(name: impl Into<String>) -> Self {
        Self::Function {
            choice_type: "function".to_string(),
            function: ToolChoiceFunction { name: name.into() },
        }
    }
}

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// The function invocation inside a [`ToolCall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    /// Raw JSON string of arguments, exactly as emitted by the model.
    pub arguments: String,
}

/// A single part of a multimodal message: either a text span or an image.
///
/// Serializes to OpenAI's "content parts" wire format, e.g.
/// `{"type":"text","text":"..."}` or
/// `{"type":"image_url","image_url":{"url":"..."}}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// A span of text.
    Text { text: String },
    /// An image referenced by URL or `data:` URI.
    ImageUrl { image_url: ImageUrl },
}

impl ContentPart {
    /// Build a text content part.
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    /// Build an image content part from a URL or `data:` URI.
    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: url.into(),
                detail: None,
            },
        }
    }
}

/// An image reference inside a [`ContentPart::ImageUrl`].
///
/// `url` may be a public `https://` URL or an inline `data:<mime>;base64,<...>`
/// URI. `detail` is an optional OpenAI hint (`"low"`, `"high"`, `"auto"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Message content: either a plain string or a list of multimodal parts
/// (text + images), matching OpenAI's `content` field.
///
/// A bare string serializes as a JSON string and a part list as a JSON array,
/// so the wire format stays byte-compatible with OpenAI in both directions.
/// All the string-based constructors on [`Message`] keep working unchanged via
/// the `From<String>` / `From<&str>` conversions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    /// Plain text content.
    Text(String),
    /// Multimodal content parts (text and/or images).
    Parts(Vec<ContentPart>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

impl Content {
    /// True if this is empty text or an empty part list.
    pub fn is_empty(&self) -> bool {
        match self {
            Content::Text(s) => s.is_empty(),
            Content::Parts(parts) => parts.is_empty(),
        }
    }

    /// Concatenate all text, ignoring image parts. Used by text-only providers
    /// and to extract system-prompt text.
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => {
                let mut out = String::new();
                for part in parts {
                    if let ContentPart::Text { text } = part {
                        out.push_str(text);
                    }
                }
                out
            }
        }
    }

    /// The image references in this content, in order.
    pub fn images(&self) -> Vec<&ImageUrl> {
        let mut out = Vec::new();
        if let Content::Parts(parts) = self {
            for part in parts {
                if let ContentPart::ImageUrl { image_url } = part {
                    out.push(image_url);
                }
            }
        }
        out
    }
}

impl From<String> for Content {
    fn from(s: String) -> Self {
        Content::Text(s)
    }
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Content::Text(s.to_string())
    }
}

impl From<Vec<ContentPart>> for Content {
    fn from(parts: Vec<ContentPart>) -> Self {
        Content::Parts(parts)
    }
}

/// A single chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Content,
    /// Tool calls requested by the assistant (present on assistant turns that
    /// invoke tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The id of the tool call this message responds to (present on `tool`
    /// messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional participant name (e.g. the tool/function name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Build a user message from multimodal content parts (text and/or images).
    pub fn user_with_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::User,
            content: Content::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Convenience: a user message pairing a text prompt with a single image
    /// (an `https://` URL or a `data:` URI).
    pub fn user_with_image(text: impl Into<String>, image_url: impl Into<String>) -> Self {
        Self::user_with_parts(vec![
            ContentPart::text(text),
            ContentPart::image_url(image_url),
        ])
    }

    /// Build a `tool` message carrying the result of a tool call, keyed by the
    /// originating [`ToolCall::id`].
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Content::Text(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }

    /// Build an assistant message that requests one or more tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::Text(String::new()),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// A complete chat completion response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
    /// Tool calls requested by the model, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The reason the model stopped generating (e.g. `"stop"`, `"length"`,
    /// `"tool_calls"`), when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// A single chunk from a streaming response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamChunk {
    /// The text delta for this chunk (may be empty for the final chunk).
    pub delta: String,
    /// Set to true when the stream is complete.
    pub done: bool,
    /// The provider-reported reason the stream stopped (e.g. `"stop"`,
    /// `"length"`), when available. Populated on the terminal content chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Token usage for the request. Only populated on the terminal usage
    /// chunk, and only for providers that report usage while streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// A chat completion request.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub stream: bool,
    pub top_p: Option<f64>,
    /// Tools the model is allowed to call.
    pub tools: Option<Vec<Tool>>,
    /// Tool-choice policy.
    pub tool_choice: Option<ToolChoice>,
}

impl ChatRequest {
    /// Create a simple single-message request.
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            messages: vec![Message::user(prompt)],
            ..Default::default()
        }
    }

    /// Add a system message at the beginning.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.messages.insert(0, Message::system(system));
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temp: f64) -> Self {
        self.temperature = Some(temp);
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max: u64) -> Self {
        self.max_tokens = Some(max);
        self
    }

    /// Enable streaming.
    pub fn with_stream(mut self) -> Self {
        self.stream = true;
        self
    }

    /// Advertise tools the model may call.
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the tool-choice policy.
    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_choice_mode_serializes_as_string() {
        assert_eq!(
            serde_json::to_value(ToolChoice::auto()).unwrap(),
            serde_json::json!("auto")
        );
        assert_eq!(
            serde_json::to_value(ToolChoice::required()).unwrap(),
            serde_json::json!("required")
        );
    }

    #[test]
    fn tool_choice_function_serializes_as_object() {
        let v = serde_json::to_value(ToolChoice::function("get_weather")).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "get_weather");
    }

    #[test]
    fn tool_serializes_with_function_schema() {
        let tool = Tool::function(
            "get_weather",
            Some("Get the weather".to_string()),
            serde_json::json!({"type": "object"}),
        );
        let v = serde_json::to_value(&tool).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "get_weather");
        assert_eq!(v["function"]["description"], "Get the weather");
        assert_eq!(v["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn tool_message_has_role_and_id() {
        let msg = Message::tool("call_1", "result");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn content_text_serializes_as_string() {
        let c = Content::Text("hi".to_string());
        assert_eq!(serde_json::to_value(&c).unwrap(), serde_json::json!("hi"));
    }

    #[test]
    fn content_parts_serialize_as_openai_array() {
        let c = Content::Parts(vec![
            ContentPart::text("look"),
            ContentPart::image_url("https://example.com/a.png"),
        ]);
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.is_array());
        assert_eq!(v[0]["type"], "text");
        assert_eq!(v[0]["text"], "look");
        assert_eq!(v[1]["type"], "image_url");
        assert_eq!(v[1]["image_url"]["url"], "https://example.com/a.png");
        assert!(v[1]["image_url"].get("detail").is_none());
    }

    #[test]
    fn content_deserializes_from_string_and_array() {
        let from_str: Content = serde_json::from_value(serde_json::json!("hi")).unwrap();
        assert_eq!(from_str, Content::Text("hi".to_string()));

        let from_arr: Content = serde_json::from_value(serde_json::json!([
            {"type": "text", "text": "a"},
            {"type": "image_url", "image_url": {"url": "u"}}
        ]))
        .unwrap();
        assert_eq!(from_arr.images().len(), 1);
        assert_eq!(from_arr.as_text(), "a");
    }

    #[test]
    fn message_user_with_image_holds_parts() {
        let msg = Message::user_with_image("desc", "https://example.com/a.png");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.images().len(), 1);
        assert_eq!(msg.content.as_text(), "desc");
    }
}
