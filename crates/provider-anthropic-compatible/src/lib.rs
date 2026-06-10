use async_trait::async_trait;
use futures::StreamExt;
use model_provider_info::ModelProviderInfo;
use model_provider_info::WireApi;
use model_runtime::ModelEventStream;
use model_runtime::ModelRuntime;
use model_runtime::ModelRuntimeError;
use model_runtime::ModelStreamEvent;
use model_runtime::ModelTurnRequest;
use model_runtime::ProviderCapabilities;
use reqwest::header::CONTENT_TYPE;
use runtime_protocol::dynamic_tools::DynamicToolSpec;
use runtime_protocol::mcp::CallToolResult;
use runtime_protocol::models::ContentItem;
use runtime_protocol::models::FunctionCallOutputPayload;
use runtime_protocol::models::ReasoningItemContent;
use runtime_protocol::models::ResponseInputItem;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::protocol::TokenUsage;
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1024;

#[derive(Debug, Clone)]
pub struct AnthropicCompatibleRuntime {
    provider: ModelProviderInfo,
    client: reqwest::Client,
}

impl AnthropicCompatibleRuntime {
    pub fn new(provider: ModelProviderInfo) -> Result<Self, ModelRuntimeError> {
        if provider.wire_api != WireApi::AnthropicCompatible {
            return Err(ModelRuntimeError::UnsupportedProvider(format!(
                "wire_api `{}` is not anthropic-compatible",
                provider.wire_api
            )));
        }
        Ok(Self {
            provider,
            client: reqwest::Client::new(),
        })
    }

    fn endpoint_url(&self) -> Result<String, ModelRuntimeError> {
        let base_url = self.provider.base_url.as_deref().ok_or_else(|| {
            ModelRuntimeError::InvalidRequest("provider.base_url is required".into())
        })?;
        let trimmed = base_url.trim_end_matches('/');
        if trimmed.ends_with("/messages") {
            Ok(trimmed.to_string())
        } else if trimmed.ends_with("/v1") {
            Ok(format!("{trimmed}/messages"))
        } else {
            Ok(format!("{trimmed}/v1/messages"))
        }
    }

    fn api_key(&self) -> Result<String, ModelRuntimeError> {
        if let Some(token) = self.provider.experimental_bearer_token.clone() {
            return Ok(token);
        }
        self.provider
            .api_key()
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?
            .ok_or_else(|| {
                ModelRuntimeError::InvalidRequest(
                    "anthropic-compatible provider requires env_key or experimental_bearer_token"
                        .into(),
                )
            })
    }

    fn anthropic_version(&self) -> &str {
        self.provider
            .http_headers
            .as_ref()
            .and_then(|headers| headers.get("anthropic-version"))
            .map(String::as_str)
            .unwrap_or(DEFAULT_ANTHROPIC_VERSION)
    }

    fn build_request_body(&self, request: &ModelTurnRequest) -> Result<Value, ModelRuntimeError> {
        let messages = request
            .messages
            .iter()
            .map(map_message)
            .collect::<Result<Vec<_>, _>>()?;

        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
            "messages": messages,
            "stream": true,
        });

        if !request.base_instructions.text.trim().is_empty() {
            body["system"] = Value::String(request.base_instructions.text.clone());
        }

        if !request.tools.is_empty() {
            body["tools"] =
                Value::Array(request.tools.iter().map(map_tool_spec).collect::<Vec<_>>());
        }

        Ok(body)
    }
}

#[async_trait]
impl ModelRuntime for AnthropicCompatibleRuntime {
    fn provider_capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tool_calling: true,
            parallel_tools: false,
            image_input: true,
            reasoning: true,
            reasoning_summaries: false,
            structured_output: false,
            context_window: None,
        }
    }

    async fn stream_turn(
        &self,
        request: ModelTurnRequest,
    ) -> Result<ModelEventStream, ModelRuntimeError> {
        let body = self.build_request_body(&request)?;
        let api_key = self.api_key()?;
        let endpoint = self.endpoint_url()?;

        let response = self
            .client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header("x-api-key", api_key)
            .header("anthropic-version", self.anthropic_version())
            .json(&body)
            .send()
            .await
            .map_err(|err| ModelRuntimeError::Http(err.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read error body>".to_string());
            return Err(ModelRuntimeError::Provider(format!("{status}: {body}")));
        }

        let mut stream = response.bytes_stream();
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut parser = SseParser::default();
            let mut state = StreamState::default();

            while let Some(next) = stream.next().await {
                match next {
                    Ok(chunk) => {
                        if let Err(err) = parser.push_chunk(&chunk, &mut state, &tx) {
                            let _ = tx.send(Err(err));
                            return;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(Err(ModelRuntimeError::Http(err.to_string())));
                        return;
                    }
                }
            }

            if let Err(err) = parser.finish(&mut state, &tx) {
                let _ = tx.send(Err(err));
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }
}

#[derive(Debug, Default)]
struct SseParser {
    pending_line: String,
    event_name: Option<String>,
    data_lines: Vec<String>,
}

impl SseParser {
    fn push_chunk(
        &mut self,
        chunk: &[u8],
        state: &mut StreamState,
        tx: &mpsc::UnboundedSender<Result<ModelStreamEvent, ModelRuntimeError>>,
    ) -> Result<(), ModelRuntimeError> {
        let text = String::from_utf8_lossy(chunk);
        self.pending_line.push_str(&text);

        while let Some(pos) = self.pending_line.find('\n') {
            let mut line = self.pending_line[..pos].to_string();
            self.pending_line.drain(..=pos);
            if line.ends_with('\r') {
                line.pop();
            }

            if line.is_empty() {
                self.flush_event(state, tx)?;
                continue;
            }

            if let Some(rest) = line.strip_prefix("event:") {
                self.event_name = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                self.data_lines.push(rest.trim_start().to_string());
            }
        }

        Ok(())
    }

    fn finish(
        &mut self,
        state: &mut StreamState,
        tx: &mpsc::UnboundedSender<Result<ModelStreamEvent, ModelRuntimeError>>,
    ) -> Result<(), ModelRuntimeError> {
        if !self.pending_line.trim().is_empty() {
            if let Some(rest) = self.pending_line.strip_prefix("data:") {
                self.data_lines.push(rest.trim_start().to_string());
            }
            self.pending_line.clear();
        }
        if !self.data_lines.is_empty() || self.event_name.is_some() {
            self.flush_event(state, tx)?;
        }
        Ok(())
    }

    fn flush_event(
        &mut self,
        state: &mut StreamState,
        tx: &mpsc::UnboundedSender<Result<ModelStreamEvent, ModelRuntimeError>>,
    ) -> Result<(), ModelRuntimeError> {
        if self.data_lines.is_empty() && self.event_name.is_none() {
            return Ok(());
        }
        let event_name = self.event_name.take();
        let data = self.data_lines.join("\n");
        self.data_lines.clear();
        process_sse_event(event_name.as_deref(), &data, state, tx)
    }
}

#[derive(Debug, Default)]
struct StreamState {
    blocks: HashMap<i64, ContentBlockState>,
    stop_reason: Option<String>,
}

#[derive(Debug)]
enum ContentBlockState {
    Text(String),
    ToolUse {
        call_id: String,
        name: String,
        input_json: String,
    },
    Reasoning(String),
}

fn process_sse_event(
    event_name: Option<&str>,
    data: &str,
    state: &mut StreamState,
    tx: &mpsc::UnboundedSender<Result<ModelStreamEvent, ModelRuntimeError>>,
) -> Result<(), ModelRuntimeError> {
    if data.trim() == "[DONE]" {
        let _ = tx.send(Ok(ModelStreamEvent::Completed {
            stop_reason: state.stop_reason.clone(),
        }));
        return Ok(());
    }

    let payload: Value = serde_json::from_str(data)
        .map_err(|err| ModelRuntimeError::StreamParse(format!("invalid json event: {err}")))?;
    let event_type = event_name
        .or_else(|| payload.get("type").and_then(Value::as_str))
        .unwrap_or("");

    match event_type {
        "ping" => {}
        "error" => {
            let message = payload
                .get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown anthropic stream error");
            return Err(ModelRuntimeError::Provider(message.to_string()));
        }
        "message_start" => {
            if let Some(usage) = payload
                .get("message")
                .and_then(|message| message.get("usage"))
                .and_then(parse_usage)
            {
                let _ = tx.send(Ok(ModelStreamEvent::Usage(usage)));
            }
        }
        "content_block_start" => {
            let index = payload
                .get("index")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    ModelRuntimeError::StreamParse("missing content block index".into())
                })?;
            let block = payload
                .get("content_block")
                .ok_or_else(|| ModelRuntimeError::StreamParse("missing content block".into()))?;
            let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
            let state_value = match block_type {
                "text" => ContentBlockState::Text(
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ),
                "tool_use" => ContentBlockState::ToolUse {
                    call_id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input_json: block
                        .get("input")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "{}".to_string()),
                },
                "thinking" => ContentBlockState::Reasoning(
                    block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ),
                _ => return Ok(()),
            };
            state.blocks.insert(index, state_value);
        }
        "content_block_delta" => {
            let index = payload
                .get("index")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    ModelRuntimeError::StreamParse("missing content block index".into())
                })?;
            let delta = payload
                .get("delta")
                .ok_or_else(|| ModelRuntimeError::StreamParse("missing delta payload".into()))?;
            let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
            match (state.blocks.get_mut(&index), delta_type) {
                (Some(ContentBlockState::Text(text)), "text_delta") => {
                    let piece = delta
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    text.push_str(&piece);
                    let _ = tx.send(Ok(ModelStreamEvent::TextDelta { text: piece }));
                }
                (
                    Some(ContentBlockState::ToolUse {
                        call_id,
                        input_json,
                        ..
                    }),
                    "input_json_delta",
                ) => {
                    let piece = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    input_json.push_str(&piece);
                    let _ = tx.send(Ok(ModelStreamEvent::ToolArgumentsDelta {
                        call_id: call_id.clone(),
                        delta: piece,
                    }));
                }
                (Some(ContentBlockState::Reasoning(text)), "thinking_delta") => {
                    let piece = delta
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    text.push_str(&piece);
                    let _ = tx.send(Ok(ModelStreamEvent::ReasoningDelta { text: piece }));
                }
                (_, "signature_delta") => {}
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = payload
                .get("index")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    ModelRuntimeError::StreamParse("missing content block index".into())
                })?;
            if let Some(block) = state.blocks.remove(&index) {
                let item = match block {
                    ContentBlockState::Text(text) => ResponseItem::Message {
                        id: None,
                        role: "assistant".to_string(),
                        content: vec![ContentItem::OutputText { text }],
                        phase: None,
                    },
                    ContentBlockState::ToolUse {
                        call_id,
                        name,
                        input_json,
                    } => ResponseItem::FunctionCall {
                        id: None,
                        name,
                        namespace: None,
                        arguments: normalize_json_string(input_json),
                        call_id,
                    },
                    ContentBlockState::Reasoning(text) => ResponseItem::Reasoning {
                        id: format!("thinking-{index}"),
                        summary: Vec::new(),
                        content: Some(vec![ReasoningItemContent::ReasoningText { text }]),
                        encrypted_content: None,
                    },
                };
                let _ = tx.send(Ok(ModelStreamEvent::ResponseItem(item)));
            }
        }
        "message_delta" => {
            if let Some(stop_reason) = payload
                .get("delta")
                .and_then(|value| value.get("stop_reason"))
                .and_then(Value::as_str)
            {
                state.stop_reason = Some(stop_reason.to_string());
            }
            if let Some(usage) = payload.get("usage").and_then(parse_usage) {
                let _ = tx.send(Ok(ModelStreamEvent::Usage(usage)));
            }
        }
        "message_stop" => {
            let _ = tx.send(Ok(ModelStreamEvent::Completed {
                stop_reason: state.stop_reason.clone(),
            }));
        }
        _ => {}
    }

    Ok(())
}

fn parse_usage(value: &Value) -> Option<TokenUsage> {
    let input_tokens = value
        .get("input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = value
        .get("output_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Some(TokenUsage {
        input_tokens,
        cached_input_tokens: 0,
        output_tokens,
        reasoning_output_tokens: 0,
        total_tokens: input_tokens + output_tokens,
    })
}

fn map_tool_spec(tool: &DynamicToolSpec) -> Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    })
}

fn map_message(item: &ResponseInputItem) -> Result<Value, ModelRuntimeError> {
    match item {
        ResponseInputItem::Message { role, content, .. } => Ok(serde_json::json!({
            "role": role,
            "content": content
                .iter()
                .map(map_content_item)
                .collect::<Result<Vec<_>, _>>()?,
        })),
        ResponseInputItem::FunctionCallOutput { call_id, output } => Ok(tool_result_message(
            call_id,
            tool_result_blocks_from_output(output)?,
            output.success,
        )),
        ResponseInputItem::CustomToolCallOutput {
            call_id, output, ..
        } => Ok(tool_result_message(
            call_id,
            tool_result_blocks_from_output(output)?,
            output.success,
        )),
        ResponseInputItem::McpToolCallOutput { call_id, output } => Ok(tool_result_message(
            call_id,
            tool_result_blocks_from_mcp(output),
            output.is_error.map(|is_error| !is_error),
        )),
        ResponseInputItem::ToolSearchOutput {
            call_id,
            status,
            execution,
            tools,
        } => Ok(tool_result_message(
            call_id,
            vec![serde_json::json!({
                "type": "text",
                "text": serde_json::json!({
                    "status": status,
                    "execution": execution,
                    "tools": tools,
                }).to_string(),
            })],
            Some(true),
        )),
    }
}

fn map_content_item(item: &ContentItem) -> Result<Value, ModelRuntimeError> {
    match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
            Ok(serde_json::json!({ "type": "text", "text": text }))
        }
        ContentItem::InputImage { image_url, .. } => map_data_url_image(image_url),
    }
}

fn map_data_url_image(image_url: &str) -> Result<Value, ModelRuntimeError> {
    let Some(rest) = image_url.strip_prefix("data:") else {
        return Err(ModelRuntimeError::InvalidRequest(
            "anthropic-compatible image inputs must be data URLs".into(),
        ));
    };
    let Some((media_type, encoded)) = rest.split_once(";base64,") else {
        return Err(ModelRuntimeError::InvalidRequest(
            "anthropic-compatible image inputs must use base64 data URLs".into(),
        ));
    };
    Ok(serde_json::json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": media_type,
            "data": encoded,
        }
    }))
}

fn tool_result_message(call_id: &str, content: Vec<Value>, success: Option<bool>) -> Value {
    let mut block = serde_json::json!({
        "type": "tool_result",
        "tool_use_id": call_id,
        "content": content,
    });
    if let Some(is_success) = success {
        block["is_error"] = Value::Bool(!is_success);
    }
    serde_json::json!({
        "role": "user",
        "content": [block],
    })
}

fn tool_result_blocks_from_output(
    output: &FunctionCallOutputPayload,
) -> Result<Vec<Value>, ModelRuntimeError> {
    if let Some(text) = output.text_content() {
        return Ok(vec![serde_json::json!({ "type": "text", "text": text })]);
    }
    if let Some(items) = output.content_items() {
        return items
            .iter()
            .map(|item| match item {
                runtime_protocol::models::FunctionCallOutputContentItem::InputText { text } => {
                    Ok(serde_json::json!({ "type": "text", "text": text }))
                }
                runtime_protocol::models::FunctionCallOutputContentItem::InputImage {
                    image_url,
                    ..
                } => map_data_url_image(image_url),
                runtime_protocol::models::FunctionCallOutputContentItem::EncryptedContent {
                    encrypted_content,
                } => Ok(serde_json::json!({
                    "type": "text",
                    "text": encrypted_content,
                })),
            })
            .collect();
    }
    Ok(vec![serde_json::json!({ "type": "text", "text": "" })])
}

fn tool_result_blocks_from_mcp(output: &CallToolResult) -> Vec<Value> {
    let mut parts = Vec::new();
    if let Some(structured) = output.structured_content.as_ref() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": structured.to_string(),
        }));
    }
    for item in &output.content {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| item.to_string());
        parts.push(serde_json::json!({
            "type": "text",
            "text": text,
        }));
    }
    if parts.is_empty() {
        parts.push(serde_json::json!({ "type": "text", "text": "" }));
    }
    parts
}

fn normalize_json_string(raw: String) -> String {
    serde_json::from_str::<Value>(&raw)
        .map(|value| value.to_string())
        .unwrap_or(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use runtime_protocol::models::BaseInstructions;
    use runtime_protocol::models::ContentItem;

    #[test]
    fn endpoint_builder_appends_messages_path() {
        let runtime = AnthropicCompatibleRuntime::new(
            ModelProviderInfo::create_anthropic_compatible_provider(Some(
                "https://example.com/anthropic".to_string(),
            )),
        )
        .expect("runtime should build");

        assert_eq!(
            runtime.endpoint_url().expect("endpoint should resolve"),
            "https://example.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn request_body_maps_user_text_message() {
        let provider = ModelProviderInfo::create_anthropic_compatible_provider(Some(
            "https://example.com/anthropic".to_string(),
        ));
        let runtime = AnthropicCompatibleRuntime::new(provider.clone()).expect("runtime");
        let mut request = ModelTurnRequest::new(provider, "claude-test");
        request.base_instructions = BaseInstructions {
            text: "System".to_string(),
        };
        request.messages.push(ResponseInputItem::Message {
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
            phase: None,
        });

        let body = runtime
            .build_request_body(&request)
            .expect("body should build");

        assert_eq!(
            body,
            serde_json::json!({
                "model": "claude-test",
                "max_tokens": 1024,
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "Hello"
                    }]
                }],
                "stream": true,
                "system": "System"
            })
        );
    }

    #[tokio::test]
    #[ignore = "requires live anthropic-compatible credentials"]
    async fn real_model_streams_text() {
        let base_url = std::env::var("SPRITE_ANTHROPIC_BASE_URL")
            .expect("SPRITE_ANTHROPIC_BASE_URL must be set");
        let model =
            std::env::var("SPRITE_ANTHROPIC_MODEL").expect("SPRITE_ANTHROPIC_MODEL must be set");
        let mut provider = ModelProviderInfo::create_anthropic_compatible_provider(Some(base_url));
        provider.env_key = Some("SPRITE_ANTHROPIC_API_KEY".to_string());

        let runtime = AnthropicCompatibleRuntime::new(provider.clone()).expect("runtime");
        let mut request = ModelTurnRequest::new(provider, model);
        request.messages.push(ResponseInputItem::Message {
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Reply with the exact text SPRITE_MODEL_OK and nothing else.".to_string(),
            }],
            phase: None,
        });
        request.max_output_tokens = Some(64);

        let mut stream = runtime
            .stream_turn(request)
            .await
            .expect("stream should start");
        let mut observed = String::new();

        while let Some(event) = stream.next().await {
            match event.expect("stream event should succeed") {
                ModelStreamEvent::TextDelta { text } => observed.push_str(&text),
                ModelStreamEvent::ResponseItem(ResponseItem::Message { content, .. }) => {
                    for item in content {
                        if let ContentItem::OutputText { text } = item {
                            observed.push_str(&text);
                        }
                    }
                }
                _ => {}
            }
        }

        assert!(
            observed.contains("SPRITE_MODEL_OK"),
            "expected live model response to contain marker, got: {observed}"
        );
    }
}
