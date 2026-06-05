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

/// A single chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
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
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Build a `tool` message carrying the result of a tool call, keyed by the
    /// originating [`ToolCall::id`].
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }

    /// Build an assistant message that requests one or more tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: String::new(),
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
}
