//! Google Gemini API provider.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::providers::stream_util::line_stream;
use crate::providers::{LlmError, Provider, ProviderConfig, Result};
use crate::types::{
    ChatRequest, ChatResponse, Content, ContentPart, FunctionCall, LogProbs, ResponseFormat,
    StreamChunk, TokenLogProb, Tool, ToolCall, ToolChoice, TopLogProb, Usage,
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
    #[serde(rename = "stopSequences", skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(rename = "candidateCount", skip_serializing_if = "Option::is_none")]
    candidate_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
    #[serde(rename = "presencePenalty", skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f64>,
    #[serde(rename = "frequencyPenalty", skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
    #[serde(rename = "responseSchema", skip_serializing_if = "Option::is_none")]
    response_schema: Option<serde_json::Value>,
    #[serde(rename = "responseLogprobs", skip_serializing_if = "Option::is_none")]
    response_logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<u32>,
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
    #[serde(
        rename = "allowedFunctionNames",
        skip_serializing_if = "Option::is_none"
    )]
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
    #[serde(default, rename = "logprobsResult")]
    logprobs_result: Option<GeminiLogprobsResult>,
}

/// Gemini log-probability result. `chosen_candidates` carries the per-token
/// log-probability of the tokens that were actually selected; `top_candidates`
/// carries the top-N alternatives at each position (when `top_logprobs` was
/// requested).
#[derive(Deserialize)]
struct GeminiLogprobsResult {
    #[serde(default, rename = "chosenCandidates")]
    chosen_candidates: Vec<GeminiLogprobCandidate>,
    #[serde(default, rename = "topCandidates")]
    top_candidates: Vec<GeminiTopCandidatesAtPosition>,
}

#[derive(Deserialize)]
struct GeminiLogprobCandidate {
    #[serde(default)]
    token: Option<String>,
    #[serde(default, rename = "logProbability")]
    log_probability: Option<f64>,
    #[serde(default)]
    bytes: Option<Vec<u8>>,
}

#[derive(Deserialize)]
struct GeminiTopCandidatesAtPosition {
    #[serde(default)]
    candidate: Vec<GeminiLogprobCandidate>,
}

/// Convert Gemini logprobs into the OpenAI-normalized [`LogProbs`] shape.
fn gemini_logprobs_to_logprobs(lp: &GeminiLogprobsResult) -> Option<LogProbs> {
    if lp.chosen_candidates.is_empty() {
        return None;
    }
    let content: Vec<TokenLogProb> = lp
        .chosen_candidates
        .iter()
        .enumerate()
        .map(|(i, chosen)| {
            let top_logprobs = lp
                .top_candidates
                .get(i)
                .map(|tc| {
                    tc.candidate
                        .iter()
                        .map(|c| TopLogProb {
                            token: c.token.clone().unwrap_or_default(),
                            logprob: c.log_probability.unwrap_or(0.0),
                            bytes: c.bytes.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            TokenLogProb {
                token: chosen.token.clone().unwrap_or_default(),
                logprob: chosen.log_probability.unwrap_or(0.0),
                bytes: chosen.bytes.clone(),
                top_logprobs,
            }
        })
        .collect();
    Some(LogProbs { content })
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

/// Accumulates streamed `functionCall` parts from a Gemini stream. Gemini emits
/// each tool call as a complete `functionCall` part (name + args), so the
/// accumulator simply collects them and drains the set once a `finishReason`
/// arrives.
#[derive(Default)]
struct GeminiToolAccumulator {
    calls: Vec<ToolCall>,
}

impl GeminiToolAccumulator {
    fn push(&mut self, name: String, args: serde_json::Value) {
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

/// Convert a tool-result string into the JSON object Gemini's `functionResponse`
/// expects. A payload that already parses as a JSON object is passed through
/// unchanged; anything else (plain text, or a non-object JSON value) is wrapped
/// as `{"result": <text>}`.
fn tool_result_response(text: &str) -> serde_json::Value {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value @ serde_json::Value::Object(_)) => value,
        _ => serde_json::json!({ "result": text }),
    }
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

/// Map an llmrust [`ResponseFormat`] to Gemini's `responseMimeType` /
/// `responseSchema`. JSON mode sets the mime type to `application/json`; a JSON
/// schema additionally sets `responseSchema` (the bare schema is extracted from
/// the OpenAI-style `{name, schema, strict}` wrapper when present).
fn gemini_response_format(format: &ResponseFormat) -> (Option<String>, Option<serde_json::Value>) {
    match format {
        ResponseFormat::Text => (None, None),
        ResponseFormat::JsonObject => (Some("application/json".to_string()), None),
        ResponseFormat::JsonSchema { json_schema } => {
            let schema = json_schema
                .get("schema")
                .cloned()
                .unwrap_or_else(|| json_schema.clone());
            (Some("application/json".to_string()), Some(schema))
        }
    }
}

/// Build Gemini's `generationConfig` from the request's sampling parameters.
/// All fields are optional and omitted when unset, so the wire format stays
/// backward compatible. OpenAI-style `logprobs` (enable) + `top_logprobs`
/// (count) map to Gemini's `responseLogprobs` + `logprobs`.
fn build_generation_config(req: &ChatRequest) -> GeminiGenerationConfig {
    let (response_mime_type, response_schema) = match &req.response_format {
        Some(format) => gemini_response_format(format),
        None => (None, None),
    };
    GeminiGenerationConfig {
        temperature: req.temperature,
        max_output_tokens: req.max_tokens,
        top_p: req.top_p,
        stop_sequences: req.stop.clone(),
        candidate_count: req.n,
        seed: req.seed,
        presence_penalty: req.presence_penalty,
        frequency_penalty: req.frequency_penalty,
        response_mime_type,
        response_schema,
        response_logprobs: req.logprobs,
        logprobs: req.top_logprobs,
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
/// [`StreamChunk`]s, threading a [`GeminiToolAccumulator`] so streamed
/// `functionCall` parts can be surfaced as tool calls. Lines are guaranteed
/// complete by [`line_stream`].
///
/// Completion is keyed off the real `finishReason` field rather than guessing
/// based on an empty chunk. The trailing event may carry only `usageMetadata`,
/// which is surfaced as a usage-only chunk. Text is surfaced as deltas;
/// `functionCall` parts are accumulated and surfaced as `tool_calls` on the
/// terminal chunk, with `finish_reason` normalized to `tool_calls`.
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

    let Some(candidate) = event.candidates.into_iter().next() else {
        if usage.is_some() {
            return vec![Ok(StreamChunk {
                usage,
                ..Default::default()
            })];
        }
        return Vec::new();
    };

    let mut finish_reason = candidate.finish_reason;
    let mut text = String::new();
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

    let tool_calls = if finish_reason.is_some() {
        tools.take()
    } else {
        None
    };
    if tool_calls.is_some() {
        finish_reason = Some("tool_calls".to_string());
    }

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
            tool_calls,
            ..Default::default()
        }));
    }
    chunks
}

/// Build the Gemini request body shared by `chat` and `stream`.
fn build_body<'a>(
    req: &ChatRequest,
    contents: &'a [GeminiContent],
    system_instruction: Option<GeminiContent>,
) -> GeminiRequest<'a> {
    GeminiRequest {
        contents,
        system_instruction,
        generation_config: Some(build_generation_config(req)),
        tools: req.tools.as_deref().map(to_gemini_tools),
        tool_config: req.tool_choice.as_ref().map(to_gemini_tool_config),
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let (contents, system_instruction) = build_contents(req);
        let body = build_body(req, &contents, system_instruction);

        let url = format!("{}/models/{}:generateContent", self.base_url, req.model);

        tracing::debug!(
            provider = "google",
            model = &req.model,
            "sending chat request"
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
            let err = LlmError::Api {
                status: status.as_u16(),
                message: msg,
            };
            tracing::error!(provider = "google", status = status.as_u16(), error = %err, "API error");
            return Err(err);
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

        let (content, tool_calls, finish_reason, logprobs) =
            match parsed.candidates.into_iter().next() {
                Some(candidate) => {
                    let candidate_finish = candidate.finish_reason;
                    let lp = candidate
                        .logprobs_result
                        .as_ref()
                        .and_then(gemini_logprobs_to_logprobs);
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
                    (text, tool_calls, finish_reason, lp)
                }
                None => (String::new(), None, None, None),
            };

        let result = ChatResponse {
            content,
            model: parsed.model_version,
            usage,
            tool_calls,
            finish_reason,
            logprobs,
        };
        tracing::debug!(provider = "google", model = &result.model, finish_reason = ?result.finish_reason, "chat response received");
        Ok(result)
    }

    async fn stream(&self, req: &ChatRequest) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (contents, system_instruction) = build_contents(req);
        let body = build_body(req, &contents, system_instruction);

        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base_url, req.model
        );

        tracing::debug!(
            provider = "google",
            model = &req.model,
            "sending stream request"
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
            let err = LlmError::Api {
                status: status.as_u16(),
                message: msg,
            };
            tracing::error!(provider = "google", status = status.as_u16(), error = %err, "API error");
            return Err(err);
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
        assert_eq!(
            v[0]["functionDeclarations"][0]["parameters"]["type"],
            "object"
        );
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
    fn generation_config_forwards_sampling_params() {
        let req = ChatRequest::new("gemini", "hi")
            .with_stop(vec!["END".to_string()])
            .with_seed(42)
            .with_n(2)
            .with_presence_penalty(0.5)
            .with_frequency_penalty(-0.3)
            .with_top_p(0.9);
        let cfg = build_generation_config(&req);
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["stopSequences"][0], "END");
        assert_eq!(v["seed"], 42);
        assert_eq!(v["candidateCount"], 2);
        assert_eq!(v["presencePenalty"], 0.5);
        assert_eq!(v["frequencyPenalty"], -0.3);
        assert_eq!(v["topP"], 0.9);
    }

    #[test]
    fn json_mode_sets_response_mime_type() {
        let req = ChatRequest::new("gemini", "hi").with_json_mode();
        let cfg = build_generation_config(&req);
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["responseMimeType"], "application/json");
        assert!(v.get("responseSchema").is_none());
    }

    #[test]
    fn json_schema_sets_response_schema() {
        let schema = serde_json::json!({"name": "person", "schema": {"type": "object"}});
        let req = ChatRequest::new("gemini", "hi")
            .with_response_format(ResponseFormat::json_schema(schema));
        let cfg = build_generation_config(&req);
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["responseMimeType"], "application/json");
        assert_eq!(v["responseSchema"]["type"], "object");
    }

    #[test]
    fn logprobs_map_to_gemini_fields() {
        let req = ChatRequest::new("gemini", "hi")
            .with_logprobs(true)
            .with_top_logprobs(5);
        let cfg = build_generation_config(&req);
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["responseLogprobs"], true);
        assert_eq!(v["logprobs"], 5);
    }

    #[test]
    fn empty_generation_config_omits_all_optional_fields() {
        let req = ChatRequest::new("gemini", "hi");
        let cfg = build_generation_config(&req);
        let v = serde_json::to_value(&cfg).unwrap();
        assert!(v.as_object().unwrap().is_empty());
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
        assert_eq!(
            v[2]["parts"][0]["functionResponse"]["response"]["result"],
            "sunny"
        );
    }

    #[test]
    fn json_tool_result_is_passed_through_as_object() {
        let v = tool_result_response("{\"temp\": 21}");
        assert_eq!(v["temp"], 21);
        let v = tool_result_response("plain text");
        assert_eq!(v["result"], "plain text");
    }

    #[test]
    fn stream_surfaces_function_call_as_tool_calls() {
        let mut tools = GeminiToolAccumulator::default();
        let lines = [
            r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"city":"SF"}}}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":2,"totalTokenCount":3}}"#,
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

    #[test]
    fn gemini_logprobs_parsed_into_logprobs() {
        let raw = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "hi"}]},
                "finishReason": "STOP",
                "logprobsResult": {
                    "chosenCandidates": [
                        {"token": "hi", "logProbability": -0.3, "bytes": [104, 105]}
                    ],
                    "topCandidates": [{
                        "candidate": [
                            {"token": "hi", "logProbability": -0.3, "bytes": [104, 105]},
                            {"token": "hello", "logProbability": -1.2}
                        ]
                    }]
                }
            }],
            "modelVersion": "gemini-2.0-flash"
        })
        .to_string();
        let parsed: GeminiResponse = serde_json::from_str(&raw).unwrap();
        let candidate = parsed.candidates.into_iter().next().unwrap();
        let lp = candidate
            .logprobs_result
            .as_ref()
            .and_then(gemini_logprobs_to_logprobs)
            .expect("logprobs should be present");
        assert_eq!(lp.content.len(), 1);
        assert_eq!(lp.content[0].token, "hi");
        assert_eq!(lp.content[0].logprob, -0.3);
        assert_eq!(lp.content[0].bytes, Some(vec![104, 105]));
        assert_eq!(lp.content[0].top_logprobs.len(), 2);
        assert_eq!(lp.content[0].top_logprobs[1].token, "hello");
    }
}
