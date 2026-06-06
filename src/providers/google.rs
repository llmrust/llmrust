//! Google Gemini API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FunctionCall, StreamChunk, Tool, ToolCall,
    ToolChoice, Usage,
};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

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

// --- Gemini request types ---

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
    },
    InlineData {
        inline_data: GeminiInlineData,
    },
    FunctionCall {
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        function_response: GeminiFunctionResponse,
    },
}

#[derive(Serialize)]
struct GeminiInlineData {
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
struct GeminiFunctionCall {
    name: String,
    args: Value,
}

#[derive(Serialize)]
struct GeminiFunctionResponse {
    name: String,
    response: Value,
}

#[derive(Serialize)]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
}

#[derive(Serialize)]
struct GeminiToolConfig {
    function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Serialize)]
struct GeminiFunctionCallingConfig {
    mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_function_names: Option<Vec<String>>,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

// --- Conversion helpers ---

fn gemini_image_part(url: &str) -> GeminiPart {
    if let Some((meta, data)) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    {
        let mime_type = meta
            .split(';')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("image/png")
            .to_string();
        return GeminiPart::InlineData {
            inline_data: GeminiInlineData {
                mime_type,
                data: data.to_string(),
            },
        };
    }
    // Gemini inline_data expects base64; for a remote URL we still pass it
    // through as data so callers relying on URL images get a clear API error
    // rather than a silent drop.
    GeminiPart::InlineData {
        inline_data: GeminiInlineData {
            mime_type: "image/png".to_string(),
            data: url.to_string(),
        },
    }
}

fn to_gemini_parts(content: &Content) -> Vec<GeminiPart> {
    match content {
        Content::Text(text) => vec![GeminiPart::Text { text: text.clone() }],
        Content::Parts(parts) => parts
            .iter()
            .map(|part| match part {
                ContentPart::Text { text } => GeminiPart::Text { text: text.clone() },
                ContentPart::ImageUrl { image_url } => gemini_image_part(&image_url.url),
            })
            .collect(),
    }
}

fn to_gemini_tools(tools: &[Tool]) -> Vec<GeminiTool> {
    let declarations = tools
        .iter()
        .map(|t| GeminiFunctionDeclaration {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            parameters: Some(t.function.parameters.clone()),
        })
        .collect();
    vec![GeminiTool {
        function_declarations: declarations,
    }]
}

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

/// Gemini uses `model` for the assistant role and folds system content into a
/// separate `system_instruction`. Tool calls map to `functionCall` parts and
/// tool results map to `functionResponse` parts.
fn build_contents(req: &ChatRequest) -> (Option<GeminiSystemInstruction>, Vec<GeminiContent>) {
    let mut system_parts: Vec<GeminiPart> = Vec::new();
    let mut contents: Vec<GeminiContent> = Vec::new();

    for msg in &req.messages {
        match msg.role {
            crate::types::Role::System => {
                system_parts.push(GeminiPart::Text {
                    text: msg.content.as_text(),
                });
            }
            crate::types::Role::User => contents.push(GeminiContent {
                role: "user".to_string(),
                parts: to_gemini_parts(&msg.content),
            }),
            crate::types::Role::Tool => contents.push(GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiPart::FunctionResponse {
                    function_response: GeminiFunctionResponse {
                        name: msg.tool_call_id.clone().unwrap_or_default(),
                        response: serde_json::json!({ "result": msg.content.as_text() }),
                    },
                }],
            }),
            crate::types::Role::Assistant => {
                let mut parts = Vec::new();
                let text = msg.content.as_text();
                if !text.is_empty() {
                    parts.push(GeminiPart::Text { text });
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for call in tool_calls {
                        let args: Value = serde_json::from_str(&call.function.arguments)
                            .unwrap_or_else(|_| serde_json::json!({}));
                        parts.push(GeminiPart::FunctionCall {
                            function_call: GeminiFunctionCall {
                                name: call.function.name.clone(),
                                args,
                            },
                        });
                    }
                }
                contents.push(GeminiContent {
                    role: "model".to_string(),
                    parts,
                });
            }
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(GeminiSystemInstruction {
            parts: system_parts,
        })
    };
    (system_instruction, contents)
}

// --- Gemini response types ---

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
    #[serde(default)]
    content: Option<GeminiContentResponse>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    #[serde(default)]
    parts: Vec<GeminiPartResponse>,
}

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
    args: Value,
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

// --- Stream types ---

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

/// Accumulates `functionCall` parts seen across a Gemini stream. Gemini emits
/// each function call as a complete object (name + args) within a streamed
/// chunk rather than as character fragments, so each one can be converted to a
/// [`ToolCall`] immediately and held until the terminal chunk.
#[derive(Default)]
struct GeminiToolAccumulator {
    calls: Vec<ToolCall>,
}

impl GeminiToolAccumulator {
    fn push(&mut self, name: String, args: Value) {
        self.calls.push(ToolCall {
            id: name.clone(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name,
                arguments: args.to_string(),
            },
        });
    }

    fn take(&mut self) -> Option<Vec<ToolCall>> {
        if self.calls.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.calls))
        }
    }
}

/// Parse one SSE line from a Gemini stream into zero or more [`StreamChunk`]s,
/// threading a [`GeminiToolAccumulator`] so streamed `functionCall` parts can
/// be surfaced as `tool_calls` on the terminal chunk. Gemini streams JSON
/// objects prefixed with `data: `; usage-only trailing events carry no
/// candidates. Lines are guaranteed complete by [`line_stream`].
fn parse_sse_line(tools: &mut GeminiToolAccumulator, line: &str) -> Vec<Result<StreamChunk>> {
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

    let mut text = String::new();
    let mut finish_reason = None;
    if let Some(candidate) = event.candidates.into_iter().next() {
        finish_reason = candidate.finish_reason;
        if let Some(content) = candidate.content {
            for part in content.parts {
                if let Some(t) = part.text {
                    text.push_str(&t);
                }
                if let Some(fc) = part.function_call {
                    tools.push(fc.name, fc.args);
                }
            }
        }
    }

    let tool_calls = if finish_reason.is_some() {
        tools.take()
    } else {
        None
    };
    // When a turn ends with tool calls Gemini may not set a distinct finish
    // reason; normalize to `tool_calls` so callers can branch uniformly.
    let finish_reason = if tool_calls.is_some() {
        Some("tool_calls".to_string())
    } else {
        finish_reason
    };

    let done = finish_reason.is_some();
    if !done && text.is_empty() && usage.is_none() {
        return Vec::new();
    }
    if text.is_empty() && !done && usage.is_some() {
        return vec![Ok(StreamChunk {
            usage,
            ..Default::default()
        })];
    }

    vec![Ok(StreamChunk {
        delta: text,
        done,
        finish_reason,
        usage,
        tool_calls,
    })]
}

impl GoogleProvider {
    fn build_body(&self, req: &ChatRequest) -> GeminiRequest {
        let (system_instruction, contents) = build_contents(req);
        let generation_config = GeminiGenerationConfig {
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            top_p: req.top_p,
            stop_sequences: req.stop.clone(),
        };
        GeminiRequest {
            contents,
            system_instruction,
            tools: req.tools.as_deref().map(to_gemini_tools),
            tool_config: req.tool_choice.as_ref().map(to_gemini_tool_config),
            generation_config: Some(generation_config),
        }
    }

    fn endpoint(&self, model: &str, method: &str) -> String {
        format!("{}/models/{}:{}", self.base_url, model, method)
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let body = self.build_body(req);
        let url = self.endpoint(&req.model, "generateContent");

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

        let candidate = parsed.candidates.into_iter().next();
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut finish_reason = None;
        if let Some(candidate) = candidate {
            finish_reason = candidate.finish_reason;
            if let Some(c) = candidate.content {
                for part in c.parts {
                    if let Some(t) = part.text {
                        content.push_str(&t);
                    }
                    if let Some(fc) = part.function_call {
                        tool_calls.push(ToolCall {
                            id: fc.name.clone(),
                            call_type: "function".to_string(),
                            function: FunctionCall {
                                name: fc.name,
                                arguments: fc.args.to_string(),
                            },
                        });
                    }
                }
            }
        }

        let tool_calls = if tool_calls.is_empty() {
            None
        } else {
            finish_reason = Some("tool_calls".to_string());
            Some(tool_calls)
        };

        Ok(ChatResponse {
            content,
            model: parsed.model_version,
            usage: parsed.usage_metadata.map(|u| Usage {
                prompt_tokens: u.prompt_token_count,
                completion_tokens: u.candidates_token_count,
                total_tokens: u.total_token_count,
            }),
            tool_calls,
            finish_reason,
        })
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let body = self.build_body(req);
        let url = format!(
            "{}?alt=sse",
            self.endpoint(&req.model, "streamGenerateContent")
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

        let stream = line_stream(byte_stream)
            .scan(GeminiToolAccumulator::default(), |tools, line_result| {
                let chunks = match line_result {
                    Ok(line) => parse_sse_line(tools, &line),
                    Err(e) => vec![Err(e)],
                };
                futures::future::ready(Some(futures::stream::iter(chunks)))
            })
            .flatten();

        Ok(stream.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn text_message_becomes_text_part() {
        let parts = to_gemini_parts(&Content::Text("hello".to_string()));
        let v = serde_json::to_value(&parts).unwrap();
        assert_eq!(v[0]["text"], "hello");
    }

    #[test]
    fn system_message_extracted_as_instruction() {
        let req = ChatRequest::from_messages(
            "gemini",
            vec![Message::system("be brief"), Message::user("hi")],
        );
        let (system, contents) = build_contents(&req);
        let v = serde_json::to_value(system.unwrap()).unwrap();
        assert_eq!(v["parts"][0]["text"], "be brief");
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].role, "user");
    }

    #[test]
    fn assistant_role_maps_to_model() {
        let req = ChatRequest::from_messages(
            "gemini",
            vec![Message::user("hi"), Message::assistant("hello there")],
        );
        let (_system, contents) = build_contents(&req);
        assert_eq!(contents[1].role, "model");
    }

    #[test]
    fn data_url_image_becomes_inline_data() {
        let part = gemini_image_part("data:image/jpeg;base64,QUJD");
        let v = serde_json::to_value(&part).unwrap();
        assert_eq!(v["inline_data"]["mime_type"], "image/jpeg");
        assert_eq!(v["inline_data"]["data"], "QUJD");
    }

    #[test]
    fn tools_serialize_as_function_declarations() {
        let tools = to_gemini_tools(&[Tool::function(
            "get_weather",
            Some("Get weather".to_string()),
            serde_json::json!({"type": "object"}),
        )]);
        let v = serde_json::to_value(&tools).unwrap();
        assert_eq!(v[0]["function_declarations"][0]["name"], "get_weather");
        assert_eq!(
            v[0]["function_declarations"][0]["description"],
            "Get weather"
        );
    }

    #[test]
    fn tool_config_maps_choice_mode() {
        let auto = serde_json::to_value(to_gemini_tool_config(&ToolChoice::auto())).unwrap();
        assert_eq!(auto["function_calling_config"]["mode"], "AUTO");
        let required = serde_json::to_value(to_gemini_tool_config(&ToolChoice::required())).unwrap();
        assert_eq!(required["function_calling_config"]["mode"], "ANY");
        let forced = serde_json::to_value(to_gemini_tool_config(&ToolChoice::function("f"))).unwrap();
        assert_eq!(forced["function_calling_config"]["mode"], "ANY");
        assert_eq!(
            forced["function_calling_config"]["allowed_function_names"][0],
            "f"
        );
    }

    #[test]
    fn response_function_call_parsed_into_tool_calls() {
        let raw = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": { "name": "get_weather", "args": {"city": "SF"} }
                    }]
                },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-1.5-pro"
        })
        .to_string();
        let parsed: GeminiResponse = serde_json::from_str(&raw).unwrap();
        let candidate = parsed.candidates.into_iter().next().unwrap();
        let part = candidate.content.unwrap().parts.into_iter().next().unwrap();
        let fc = part.function_call.unwrap();
        assert_eq!(fc.name, "get_weather");
        assert_eq!(fc.args["city"], "SF");
    }

    #[test]
    fn assistant_tool_calls_and_results_serialize() {
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
        let (_system, contents) = build_contents(&req);
        let v = serde_json::to_value(&contents).unwrap();
        assert_eq!(v[1]["role"], "model");
        assert_eq!(v[1]["parts"][0]["function_call"]["name"], "get_weather");
        assert_eq!(v[1]["parts"][0]["function_call"]["args"]["city"], "SF");
        assert_eq!(v[2]["role"], "user");
        assert_eq!(
            v[2]["parts"][0]["function_response"]["name"],
            "get_weather"
        );
    }

    #[test]
    fn stream_surfaces_function_call_as_tool_calls() {
        let mut tools = GeminiToolAccumulator::default();
        let lines = [
            r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"city":"SF"}}}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":4,"totalTokenCount":7}}"#,
        ];
        let mut final_chunk = None;
        for line in lines {
            for chunk in parse_sse_line(&mut tools, line) {
                final_chunk = Some(chunk.unwrap());
            }
        }
        let chunk = final_chunk.expect("a terminal chunk");
        assert!(chunk.done);
        assert_eq!(chunk.finish_reason.as_deref(), Some("tool_calls"));
        let calls = chunk.tool_calls.expect("tool calls present");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments, "{\"city\":\"SF\"}");
    }

    #[test]
    fn stream_text_chunk_has_no_tool_calls() {
        let mut tools = GeminiToolAccumulator::default();
        let mut chunks = Vec::new();
        for line in [
            r#"data: {"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}]}"#,
        ] {
            for chunk in parse_sse_line(&mut tools, line) {
                chunks.push(chunk.unwrap());
            }
        }
        assert_eq!(chunks[0].delta, "Hello");
        let last = chunks.last().unwrap();
        assert!(last.done);
        assert_eq!(last.finish_reason.as_deref(), Some("STOP"));
        assert!(last.tool_calls.is_none());
    }
}
