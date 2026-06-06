//! Google Gemini API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FunctionCall, StreamChunk, Tool, ToolCall,
    ToolChoice, Usage,
};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Build an HTTP client with explicit request/connect timeouts so a stalled
/// connection can never block a caller indefinitely. Falls back to the default
/// client if the builder fails (it will not in practice).
fn build_http_client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub struct GoogleProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl GoogleProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: build_http_client(),
            api_key: config.api_key,
            base_url: config
                .base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        }
    }
}

// --- Gemini API types ---

#[derive(Serialize)]
struct GeminiRequest<'a> {
    contents: &'a [GeminiContent],
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
}

#[derive(Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

/// A Gemini content part: text, inline binary (e.g. an image), an outgoing
/// `functionCall` (assistant tool call), or a `functionResponse` (tool result).
#[derive(Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: GeminiInlineData,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCallOut,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponseOut,
    },
}

#[derive(Serialize)]
struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
struct GeminiFunctionCallOut {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize)]
struct GeminiFunctionResponseOut {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
}

/// A Gemini tool is a bundle of function declarations.
#[derive(Serialize)]
struct GeminiTool {
    #[serde(rename = "functionDeclarations")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct GeminiToolConfig {
    #[serde(rename = "functionCallingConfig")]
    function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Serialize)]
struct GeminiFunctionCallingConfig {
    mode: String,
    #[serde(rename = "allowedFunctionNames", skip_serializing_if = "Option::is_none")]
    allowed_function_names: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "modelVersion")]
    model_version: String,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContentResponse,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    #[serde(default)]
    parts: Vec<GeminiPartResponse>,
}

/// A response part. `text` and `functionCall` are mutually exclusive in
/// practice; both are optional so any part shape deserializes cleanly (a
/// required `text` field previously crashed on `functionCall`-only parts).
#[derive(Deserialize)]
struct GeminiPartResponse {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "functionCall")]
    function_call: Option<GeminiFunctionCallResponse>,
}

#[derive(Deserialize)]
struct GeminiFunctionCallResponse {
    name: String,
    #[serde(default)]
    args: serde_json::Value,
}

#[derive(Deserialize)]
struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: u64,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: u64,
    #[serde(default, rename = "totalTokenCount")]
    total_token_count: u64,
}

#[derive(Deserialize)]
struct GeminiErrorBody {
    error: GeminiErrorDetail,
}

#[derive(Deserialize)]
struct GeminiErrorDetail {
    message: String,
}

// Stream SSE event
#[derive(Deserialize)]
struct GeminiStreamEvent {
    #[serde(default)]
    candidates: Vec<GeminiStreamCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
struct GeminiStreamCandidate {
    #[serde(default)]
    content: Option<GeminiContentResponse>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

/// Map an llmrust role to a Gemini `role`. Gemini only understands `user` and
/// `model`; system prompts are delivered separately via `systemInstruction`,
/// so a `System` message has no inline role here. Tool results are folded back
/// in as `user` turns.
fn map_gemini_role(role: &crate::types::Role) -> Option<String> {
    match role {
        crate::types::Role::System => None,
        crate::types::Role::User => Some("user".to_string()),
        crate::types::Role::Assistant => Some("model".to_string()),
        crate::types::Role::Tool => Some("user".to_string()),
    }
}

/// Convert llmrust [`Content`] into Gemini parts. Text becomes a text part;
/// images carried as `data:` URLs become `inlineData` parts. Gemini's inline
/// API cannot fetch remote URLs, so http(s) image URLs are skipped. If the
/// conversion yields nothing, an empty text part is added so the turn stays
/// well-formed.
fn content_to_parts(content: &Content) -> Vec<GeminiPart> {
    let mut parts = Vec::new();
    match content {
        Content::Text(text) => parts.push(GeminiPart::Text { text: text.clone() }),
        Content::Parts(items) => {
            for item in items {
                match item {
                    ContentPart::Text { text } => {
                        parts.push(GeminiPart::Text { text: text.clone() });
                    }
                    ContentPart::ImageUrl { image_url } => {
                        if let Some(inline_data) = gemini_inline_from_url(&image_url.url) {
                            parts.push(GeminiPart::InlineData { inline_data });
                        }
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        parts.push(GeminiPart::Text {
            text: String::new(),
        });
    }
    parts
}

/// Extract Gemini inline image data from a `data:` URL. Returns `None` for any
/// non-data URL, since Gemini's `inlineData` requires the bytes inline.
fn gemini_inline_from_url(url: &str) -> Option<GeminiInlineData> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime_type = meta
        .split(';')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("image/png")
        .to_string();
    Some(GeminiInlineData {
        mime_type,
        data: data.to_string(),
    })
}

/// Map llmrust tools to a single Gemini tool carrying all function
/// declarations.
fn to_gemini_tools(tools: &[Tool]) -> Vec<GeminiTool> {
    vec![GeminiTool {
        function_declarations: tools
            .iter()
            .map(|t| GeminiFunctionDeclaration {
                name: t.function.name.clone(),
                description: t.function.description.clone(),
                parameters: t.function.parameters.clone(),
            })
            .collect(),
    }]
}

/// Map llmrust [`ToolChoice`] to Gemini's `functionCallingConfig`. `auto`
/// stays `AUTO`, `required` becomes `ANY`, `none` becomes `NONE`, and a forced
/// function becomes `ANY` restricted to that single name.
fn to_gemini_tool_config(choice: &ToolChoice) -> GeminiToolConfig {
    let (mode, allowed) = match choice {
        ToolChoice::Mode(mode) => match mode.as_str() {
            "required" => ("ANY".to_string(), None),
            "none" => ("NONE".to_string(), None),
            _ => ("AUTO".to_string(), None),
        },
        ToolChoice::Function { function, .. } => {
            ("ANY".to_string(), Some(vec![function.name.clone()]))
        }
    };
    GeminiToolConfig {
        function_calling_config: GeminiFunctionCallingConfig {
            mode,
            allowed_function_names: allowed,
        },
    }
}

/// Wrap a tool result string into the JSON object Gemini's `functionResponse`
/// expects. If the result already parses as a JSON object it is used directly;
/// otherwise it is wrapped as `{ "result": <text> }`.
fn tool_result_response(content: &str) -> serde_json::Value {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        _ => serde_json::json!({ "result": content }),
    }
}

/// Build Gemini `contents` (conversation turns) and an optional
/// `systemInstruction` from the request messages. System messages are
/// collected into the dedicated `systemInstruction` field; assistant tool
/// calls become `functionCall` parts on a `model` turn; tool results become
/// `functionResponse` parts on a `user` turn.
fn build_contents(req: &ChatRequest) -> (Vec<GeminiContent>, Option<GeminiContent>) {
    let mut contents = Vec::new();
    let mut system_parts: Vec<GeminiPart> = Vec::new();

    for msg in &req.messages {
        match msg.role {
            crate::types::Role::System => system_parts.push(GeminiPart::Text {
                text: msg.content.as_text(),
            }),
            crate::types::Role::Tool => {
                let name = msg
                    .name
                    .clone()
                    .or_else(|| msg.tool_call_id.clone())
                    .unwrap_or_default();
                contents.push(GeminiContent {
                    parts: vec![GeminiPart::FunctionResponse {
                        function_response: GeminiFunctionResponseOut {
                            name,
                            response: tool_result_response(&msg.content.as_text()),
                        },
                    }],
                    role: Some("user".to_string()),
                });
            }
            crate::types::Role::Assistant
                if msg.tool_calls.as_ref().is_some_and(|c| !c.is_empty()) =>
            {
                let mut parts = Vec::new();
                let text = msg.content.as_text();
                if !text.is_empty() {
                    parts.push(GeminiPart::Text { text });
                }
                for call in msg.tool_calls.as_ref().unwrap() {
                    let args: serde_json::Value = serde_json::from_str(&call.function.arguments)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    parts.push(GeminiPart::FunctionCall {
                        function_call: GeminiFunctionCallOut {
                            name: call.function.name.clone(),
                            args,
                        },
                    });
                }
                contents.push(GeminiContent {
                    parts,
                    role: Some("model".to_string()),
                });
            }
            _ => contents.push(GeminiContent {
                parts: content_to_parts(&msg.content),
                role: map_gemini_role(&msg.role),
            }),
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(GeminiContent {
            parts: system_parts,
            role: None,
        })
    };

    (contents, system_instruction)
}

/// Parse a single SSE line from a Gemini stream into zero or more
/// [`StreamChunk`]s. Lines are guaranteed complete by [`line_stream`].
///
/// Completion is keyed off the real `finishReason` field rather than guessing
/// based on an empty chunk. The trailing event may carry only `usageMetadata`,
/// which is surfaced as a usage-only chunk. Streaming surfaces text deltas
/// only; tool calls are not reconstructed from the stream, so callers that
/// need tool calls should use the non-streaming `chat` path.
fn parse_sse_line(line: &str) -> Vec<Result<StreamChunk>> {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    let Ok(event) = serde_json::from_str::<GeminiStreamEvent>(data) else {
        return Vec::new();
    };

    let usage = event.usage_metadata.map(|u| Usage {
        prompt_tokens: u.prompt_token_count,
        completion_tokens: u.candidates_token_count,
        total_tokens: u.total_token_count,
    });

    let Some(candidate) = event.candidates.into_iter().next() else {
        if usage.is_some() {
            return vec![Ok(StreamChunk {
                usage,
                ..Default::default()
            })];
        }
        return Vec::new();
    };

    let finish_reason = candidate.finish_reason;
    let text = candidate
        .content
        .map(|c| {
            c.parts
                .into_iter()
                .filter_map(|p| p.text)
                .collect::<String>()
        })
        .unwrap_or_default();

    let mut chunks = Vec::new();
    if !text.is_empty() {
        chunks.push(Ok(StreamChunk {
            delta: text,
            ..Default::default()
        }));
    }
    if finish_reason.is_some() || usage.is_some() {
        chunks.push(Ok(StreamChunk {
            done: finish_reason.is_some(),
            finish_reason,
            usage,
            ..Default::default()
        }));
    }
    chunks
}

impl GoogleProvider {
    /// Build the Gemini request body shared by `chat` and `stream`.
    fn build_body<'a>(
        &self,
        req: &ChatRequest,
        contents: &'a [GeminiContent],
        system_instruction: Option<GeminiContent>,
    ) -> GeminiRequest<'a> {
        GeminiRequest {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature: req.temperature,
                max_output_tokens: req.max_tokens,
                top_p: req.top_p,
            }),
            tools: req.tools.as_deref().map(to_gemini_tools),
            tool_config: req.tool_choice.as_ref().map(to_gemini_tool_config),
        }
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let (contents, system_instruction) = build_contents(req);
        let body = self.build_body(req, &contents, system_instruction);

        let url = format!("{}/models/{}:generateContent", self.base_url, req.model);

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<GeminiErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let parsed: GeminiResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let usage = parsed.usage_metadata.map(|u| Usage {
            prompt_tokens: u.prompt_token_count,
            completion_tokens: u.candidates_token_count,
            total_tokens: u.total_token_count,
        });

        let (content, tool_calls, finish_reason) = match parsed.candidates.into_iter().next() {
            Some(candidate) => {
                let candidate_finish = candidate.finish_reason;
                let mut text = String::new();
                let mut calls: Vec<ToolCall> = Vec::new();
                for part in candidate.content.parts {
                    if let Some(t) = part.text {
                        text.push_str(&t);
                    }
                    if let Some(fc) = part.function_call {
                        let arguments = fc.args.to_string();
                        calls.push(ToolCall {
                            id: fc.name.clone(),
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: fc.name,
                                arguments,
                            },
                        });
                    }
                }
                let tool_calls = if calls.is_empty() { None } else { Some(calls) };
                let finish_reason = if tool_calls.is_some() {
                    Some("tool_calls".to_string())
                } else {
                    candidate_finish
                };
                (text, tool_calls, finish_reason)
            }
            None => (String::new(), None, None),
        };

        Ok(ChatResponse {
            content,
            model: parsed.model_version,
            usage,
            tool_calls,
            finish_reason,
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (contents, system_instruction) = build_contents(req);
        let body = self.build_body(req, &contents, system_instruction);

        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base_url, req.model
        );

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<GeminiErrorBody>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: msg,
            });
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| LlmError::Stream(e.to_string())));

        let stream = line_stream(byte_stream).flat_map(|line_result| {
            let chunks = match line_result {
                Ok(line) => parse_sse_line(&line),
                Err(e) => vec![Err(e)],
            };
            futures::stream::iter(chunks)
        });

        Ok(stream.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn text_content_maps_to_text_part() {
        let parts = content_to_parts(&Content::Text("hi".to_string()));
        let v = serde_json::to_value(&parts).unwrap();
        assert_eq!(v[0]["text"], "hi");
    }

    #[test]
    fn data_url_maps_to_inline_data() {
        let parts = content_to_parts(&Content::Parts(vec![
            ContentPart::text("look"),
            ContentPart::image_url("data:image/png;base64,QUJD"),
        ]));
        let v = serde_json::to_value(&parts).unwrap();
        assert_eq!(v[0]["text"], "look");
        assert_eq!(v[1]["inlineData"]["mimeType"], "image/png");
        assert_eq!(v[1]["inlineData"]["data"], "QUJD");
    }

    #[test]
    fn http_image_url_is_skipped() {
        let parts = content_to_parts(&Content::Parts(vec![ContentPart::image_url(
            "https://example.com/cat.png",
        )]));
        let v = serde_json::to_value(&parts).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["text"], "");
    }

    #[test]
    fn function_declarations_serialize() {
        let tools = to_gemini_tools(&[Tool::function(
            "get_weather",
            Some("w".to_string()),
            serde_json::json!({"type": "object"}),
        )]);
        let v = serde_json::to_value(&tools).unwrap();
        assert_eq!(v[0]["functionDeclarations"][0]["name"], "get_weather");
        assert_eq!(v[0]["functionDeclarations"][0]["parameters"]["type"], "object");
    }

    #[test]
    fn tool_choice_maps_to_gemini_mode() {
        let cfg = to_gemini_tool_config(&ToolChoice::required());
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["functionCallingConfig"]["mode"], "ANY");

        let cfg = to_gemini_tool_config(&ToolChoice::function("f"));
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["functionCallingConfig"]["mode"], "ANY");
        assert_eq!(v["functionCallingConfig"]["allowedFunctionNames"][0], "f");

        let cfg = to_gemini_tool_config(&ToolChoice::none());
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["functionCallingConfig"]["mode"], "NONE");
    }

    #[test]
    fn response_function_call_part_parsed() {
        let raw = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "name": "get_weather", "args": {"city": "SF"} } }] },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-1.5-pro"
        })
        .to_string();
        let parsed: GeminiResponse = serde_json::from_str(&raw).unwrap();
        let part = &parsed.candidates[0].content.parts[0];
        assert!(part.text.is_none());
        let fc = part.function_call.as_ref().unwrap();
        assert_eq!(fc.name, "get_weather");
        assert_eq!(fc.args["city"], "SF");
    }

    #[test]
    fn tool_messages_build_function_call_and_response() {
        let req = ChatRequest::from_messages(
            "gemini",
            vec![
                Message::user("weather?"),
                Message::assistant_tool_calls(vec![ToolCall {
                    id: "get_weather".to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: "{\"city\":\"SF\"}".to_string(),
                    },
                }]),
                Message::tool("get_weather", "sunny"),
            ],
        );
        let (contents, _system) = build_contents(&req);
        let v = serde_json::to_value(&contents).unwrap();
        assert_eq!(v[1]["role"], "model");
        assert_eq!(v[1]["parts"][0]["functionCall"]["name"], "get_weather");
        assert_eq!(v[1]["parts"][0]["functionCall"]["args"]["city"], "SF");
        assert_eq!(v[2]["role"], "user");
        assert_eq!(v[2]["parts"][0]["functionResponse"]["name"], "get_weather");
        assert_eq!(v[2]["parts"][0]["functionResponse"]["response"]["result"], "sunny");
    }

    #[test]
    fn json_tool_result_is_passed_through_as_object() {
        let v = tool_result_response("{\"temp\": 21}");
        assert_eq!(v["temp"], 21);
        let v = tool_result_response("plain text");
        assert_eq!(v["result"], "plain text");
    }
}
