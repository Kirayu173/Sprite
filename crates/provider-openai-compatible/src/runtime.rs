use async_trait::async_trait;
use futures::StreamExt;
use http::HeaderMap;
use http::HeaderValue;
use model_provider_info::ModelProviderInfo;
use model_provider_info::WireApi;
use model_runtime::ModelEventStream;
use model_runtime::ModelRuntime;
use model_runtime::ModelRuntimeError;
use model_runtime::ModelStreamEvent;
use model_runtime::ModelTurnRequest;
use model_runtime::ProviderCapabilities;
use model_runtime::transport::HttpRequest;
use model_runtime::transport::HttpTransport;
use model_runtime::transport::ReqwestTransport;
use model_runtime::transport::ReqwestWebsocketTransport;
use model_runtime::transport::RetryPolicy;
use model_runtime::transport::SseEvent;
use model_runtime::transport::TransportError;
use model_runtime::transport::WebsocketTransport;
use model_runtime::transport::WsMessage;
use model_runtime::transport::WsRequest;
use model_runtime::transport::run_with_retry;
use model_runtime::transport::sse_event_stream;
use runtime_protocol::dynamic_tools::DynamicToolSpec;
use runtime_protocol::models::ContentItem;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::protocol::TokenUsage;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use url::Url;

use crate::headers::build_sse_headers;
use crate::headers::build_ws_headers;

#[derive(Clone)]
pub struct OpenAiCompatibleRuntime {
    provider: ModelProviderInfo,
    http_transport: Arc<dyn HttpTransport>,
    websocket_transport: Arc<dyn WebsocketTransport>,
    capabilities: ProviderCapabilities,
}

impl OpenAiCompatibleRuntime {
    pub fn new(provider: ModelProviderInfo) -> Result<Self, ModelRuntimeError> {
        Self::new_with_transports_and_capabilities(
            provider,
            Arc::new(ReqwestTransport::new()),
            Arc::new(ReqwestWebsocketTransport),
            default_capabilities(),
        )
    }

    pub fn new_with_http_transport(
        provider: ModelProviderInfo,
        http_transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, ModelRuntimeError> {
        Self::new_with_transports_and_capabilities(
            provider,
            http_transport,
            Arc::new(ReqwestWebsocketTransport),
            default_capabilities(),
        )
    }

    pub fn new_with_transport(
        provider: ModelProviderInfo,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, ModelRuntimeError> {
        Self::new_with_http_transport(provider, transport)
    }

    pub fn new_with_transport_and_capabilities(
        provider: ModelProviderInfo,
        transport: Arc<dyn HttpTransport>,
        capabilities: ProviderCapabilities,
    ) -> Result<Self, ModelRuntimeError> {
        Self::new_with_transports_and_capabilities(
            provider,
            transport,
            Arc::new(ReqwestWebsocketTransport),
            capabilities,
        )
    }

    pub fn new_with_transports_and_capabilities(
        provider: ModelProviderInfo,
        http_transport: Arc<dyn HttpTransport>,
        websocket_transport: Arc<dyn WebsocketTransport>,
        capabilities: ProviderCapabilities,
    ) -> Result<Self, ModelRuntimeError> {
        if provider.wire_api != WireApi::Responses {
            return Err(ModelRuntimeError::UnsupportedProvider(format!(
                "wire_api `{}` is not responses-compatible",
                provider.wire_api
            )));
        }
        Ok(Self {
            provider,
            http_transport,
            websocket_transport,
            capabilities,
        })
    }

    pub(crate) fn endpoint_url(&self, path: &str) -> Result<String, ModelRuntimeError> {
        let api_provider = self
            .provider
            .to_api_provider()
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        let trimmed = api_provider.base_url.trim_end_matches('/');
        let mut url = Url::parse(&format!("{trimmed}/{path}"))
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        if let Some(query_params) = api_provider.query_params {
            let mut query = url.query_pairs_mut();
            for (key, value) in query_params {
                query.append_pair(&key, &value);
            }
        }
        Ok(url.to_string())
    }

    fn websocket_endpoint_url(&self, path: &str) -> Result<String, ModelRuntimeError> {
        let endpoint = self.endpoint_url(path)?;
        let mut url = Url::parse(&endpoint)
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        let current_scheme = url.scheme().to_string();
        let scheme = match current_scheme.as_str() {
            "http" => "ws",
            "https" => "wss",
            _ => current_scheme.as_str(),
        };
        let _ = url.set_scheme(scheme);
        Ok(url.to_string())
    }

    fn retry_policy(&self) -> Result<RetryPolicy, ModelRuntimeError> {
        let provider = self
            .provider
            .to_api_provider()
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        Ok(RetryPolicy {
            max_attempts: provider.retry.max_attempts,
            base_delay: provider.retry.base_delay,
            retry_429: provider.retry.retry_429,
            retry_5xx: provider.retry.retry_5xx,
            retry_transport: provider.retry.retry_transport,
        })
    }

    pub(crate) fn base_headers(&self) -> Result<HeaderMap, ModelRuntimeError> {
        let provider = self
            .provider
            .to_api_provider()
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        Ok(provider.headers)
    }

    fn stream_idle_timeout(&self) -> Result<std::time::Duration, ModelRuntimeError> {
        let provider = self
            .provider
            .to_api_provider()
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        Ok(provider.stream_idle_timeout)
    }

    fn websocket_connect_timeout(&self) -> std::time::Duration {
        self.provider.websocket_connect_timeout()
    }

    pub(crate) async fn auth_header(&self) -> Result<Option<HeaderValue>, ModelRuntimeError> {
        let token = resolve_bearer_token(&self.provider).await?;
        match token {
            Some(token) => HeaderValue::from_str(&format!("Bearer {token}"))
                .map(Some)
                .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string())),
            None => Ok(None),
        }
    }

    fn build_request_body(
        &self,
        request: &ModelTurnRequest,
        previous_response_id: Option<String>,
        client_metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Value {
        let mut input = request
            .messages
            .iter()
            .cloned()
            .map(ResponseItem::from)
            .collect::<Vec<_>>();

        if let Some(inter_agent_communication) = &request.inter_agent_communication {
            input.insert(0, inter_agent_communication.to_model_input_item());
        }

        let mut body = serde_json::json!({
            "model": request.model,
            "instructions": request.base_instructions.text,
            "input": input,
            "stream": true,
            "store": false,
        });

        if !request.tools.is_empty() {
            body["tools"] =
                Value::Array(request.tools.iter().map(map_tool_spec).collect::<Vec<_>>());
            body["tool_choice"] = Value::String("auto".to_string());
            body["parallel_tool_calls"] = Value::Bool(request.allow_parallel_tool_calls);
        }

        if request.reasoning_effort.is_some() || request.reasoning_summary.is_some() {
            let mut reasoning = serde_json::Map::new();
            if let Some(effort) = request.reasoning_effort.clone() {
                reasoning.insert("effort".to_string(), serde_json::json!(effort));
            }
            if let Some(summary) = request.reasoning_summary {
                reasoning.insert("summary".to_string(), serde_json::json!(summary));
            }
            body["reasoning"] = Value::Object(reasoning);
        }

        if request.verbosity.is_some() || request.output_schema.is_some() {
            let mut text = serde_json::Map::new();
            if let Some(verbosity) = request.verbosity {
                text.insert("verbosity".to_string(), serde_json::json!(verbosity));
            }
            if let Some(schema) = request.output_schema.clone() {
                text.insert(
                    "format".to_string(),
                    serde_json::json!({
                        "type": "json_schema",
                        "name": "structured_output",
                        "schema": schema,
                        "strict": request.output_schema_strict,
                    }),
                );
            }
            body["text"] = Value::Object(text);
        }

        if let Some(max_output_tokens) = request.max_output_tokens {
            body["max_output_tokens"] = serde_json::json!(max_output_tokens);
        }

        if let Some(previous_response_id) = previous_response_id {
            body["previous_response_id"] = Value::String(previous_response_id);
        }

        if let Some(client_metadata) = client_metadata
            && !client_metadata.is_empty()
        {
            body["client_metadata"] = serde_json::json!(client_metadata);
        }

        body
    }

    fn websocket_request_body(
        &self,
        request: &ModelTurnRequest,
        previous_response_id: Option<String>,
        client_metadata: Option<std::collections::HashMap<String, String>>,
    ) -> Value {
        let mut body = self.build_request_body(request, previous_response_id, client_metadata);
        let object = body
            .as_object_mut()
            .expect("response request body should always be an object");
        object.insert(
            "type".to_string(),
            Value::String("response.create".to_string()),
        );
        body
    }
}

#[async_trait]
impl ModelRuntime for OpenAiCompatibleRuntime {
    fn provider_capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    async fn stream_turn(
        &self,
        request: ModelTurnRequest,
    ) -> Result<ModelEventStream, ModelRuntimeError> {
        if self.provider.supports_websockets {
            self.stream_turn_ws(request).await
        } else {
            self.stream_turn_http(request).await
        }
    }
}

impl OpenAiCompatibleRuntime {
    async fn stream_turn_http(
        &self,
        request: ModelTurnRequest,
    ) -> Result<ModelEventStream, ModelRuntimeError> {
        let endpoint = self.endpoint_url("responses")?;
        let auth_header = self.auth_header().await?;
        let request_metadata = build_sse_headers(self.base_headers()?, auth_header, &request)?;
        let request_body = self.build_request_body(
            &request,
            request_metadata.previous_response_id.clone(),
            request_metadata.client_metadata.clone(),
        );
        let retry_policy = self.retry_policy()?;
        let idle_timeout = self.stream_idle_timeout()?;
        let transport = self.http_transport.clone();
        let headers = request_metadata.headers.clone();

        let stream_response = run_with_retry(&retry_policy, |_| {
            let transport = transport.clone();
            let headers = headers.clone();
            let endpoint = endpoint.clone();
            let request_body = request_body.clone();
            async move {
                transport
                    .stream(HttpRequest::post_json(endpoint, headers, request_body))
                    .await
            }
        })
        .await
        .map_err(map_transport_error)?;

        let mut sse = sse_event_stream(stream_response.bytes, idle_timeout);
        build_model_event_stream_from_sse(&request, &mut sse)
    }

    async fn stream_turn_ws(
        &self,
        request: ModelTurnRequest,
    ) -> Result<ModelEventStream, ModelRuntimeError> {
        let endpoint = self.websocket_endpoint_url("responses")?;
        let retry_policy = self.retry_policy()?;
        let idle_timeout = self.stream_idle_timeout()?;
        let auth_header = self.auth_header().await?;
        let request_metadata = build_ws_headers(self.base_headers()?, auth_header, &request)?;
        let request_body = self.websocket_request_body(
            &request,
            request_metadata.previous_response_id.clone(),
            request_metadata.client_metadata.clone(),
        );
        let websocket_transport = self.websocket_transport.clone();
        let headers = request_metadata.headers.clone();
        let connect_timeout = self.websocket_connect_timeout();

        let mut connected = run_with_retry(&retry_policy, |_| {
            let websocket_transport = websocket_transport.clone();
            let headers = headers.clone();
            let endpoint = endpoint.clone();
            async move {
                websocket_transport
                    .connect(WsRequest {
                        url: endpoint,
                        headers,
                        connect_timeout: Some(connect_timeout),
                    })
                    .await
            }
        })
        .await
        .map_err(map_transport_error)?;

        let request_text = serde_json::to_string(&request_body)
            .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
        timeout(idle_timeout, connected.connection.send_text(request_text))
            .await
            .map_err(|_| ModelRuntimeError::StreamInterrupted("idle timeout".to_string()))?
            .map_err(map_transport_error)?;

        let structured_output_requested = request.output_schema.is_some();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut messages = connected.messages;
            let mut saw_completed = false;

            while let Some(message) = messages.next().await {
                match message {
                    Ok(WsMessage::Text(payload)) => {
                        match process_stream_payload(&payload, structured_output_requested) {
                            Ok(events) => {
                                for model_event in events {
                                    if matches!(model_event, ModelStreamEvent::Completed { .. }) {
                                        saw_completed = true;
                                    }
                                    if tx.send(Ok(model_event)).is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(Err(error));
                                return;
                            }
                        }
                    }
                    Ok(WsMessage::Close { .. }) => {
                        let _ = tx.send(Err(ModelRuntimeError::StreamInterrupted(
                            "websocket closed before completion".to_string(),
                        )));
                        return;
                    }
                    Ok(WsMessage::Binary(_)) | Ok(WsMessage::Ping(_)) | Ok(WsMessage::Pong(_)) => {}
                    Err(error) => {
                        let _ = tx.send(Err(map_transport_error(error)));
                        return;
                    }
                }
            }

            if !saw_completed {
                let _ = tx.send(Err(ModelRuntimeError::StreamInterrupted(
                    "stream closed before completion".to_string(),
                )));
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }
}

fn build_model_event_stream_from_sse(
    request: &ModelTurnRequest,
    sse: &mut tokio_stream::wrappers::UnboundedReceiverStream<Result<SseEvent, TransportError>>,
) -> Result<ModelEventStream, ModelRuntimeError> {
    let structured_output_requested = request.output_schema.is_some();
    let (tx, rx) = mpsc::unbounded_channel();
    let mut sse = std::mem::replace(
        sse,
        UnboundedReceiverStream::new(mpsc::unbounded_channel().1),
    );

    tokio::spawn(async move {
        let mut saw_completed = false;

        while let Some(event) = sse.next().await {
            match event {
                Ok(event) => match process_sse_event(event, structured_output_requested) {
                    Ok(events) => {
                        for model_event in events {
                            if matches!(model_event, ModelStreamEvent::Completed { .. }) {
                                saw_completed = true;
                            }
                            if tx.send(Ok(model_event)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        return;
                    }
                },
                Err(error) => {
                    let _ = tx.send(Err(map_transport_error(error)));
                    return;
                }
            }
        }

        if !saw_completed {
            let _ = tx.send(Err(ModelRuntimeError::StreamInterrupted(
                "stream closed before completion".to_string(),
            )));
        }
    });

    Ok(Box::pin(UnboundedReceiverStream::new(rx)))
}

fn default_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        tool_calling: true,
        parallel_tools: true,
        image_input: true,
        reasoning: true,
        reasoning_summaries: true,
        structured_output: true,
        context_window: None,
    }
}

#[derive(Debug, Deserialize)]
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    response: Option<Value>,
    item: Option<Value>,
    item_id: Option<String>,
    call_id: Option<String>,
    delta: Option<String>,
    summary_index: Option<i64>,
}

fn process_sse_event(
    event: SseEvent,
    structured_output_requested: bool,
) -> Result<Vec<ModelStreamEvent>, ModelRuntimeError> {
    process_stream_payload(&event.data, structured_output_requested)
}

fn process_stream_payload(
    payload: &str,
    structured_output_requested: bool,
) -> Result<Vec<ModelStreamEvent>, ModelRuntimeError> {
    let payload: ResponsesStreamEvent = serde_json::from_str(payload)
        .map_err(|err| ModelRuntimeError::StreamParse(err.to_string()))?;

    match payload.kind.as_str() {
        "response.output_text.delta" => Ok(payload
            .delta
            .map(|text| vec![ModelStreamEvent::TextDelta { text }])
            .unwrap_or_default()),
        "response.reasoning_text.delta" => Ok(payload
            .delta
            .map(|text| vec![ModelStreamEvent::ReasoningDelta { text }])
            .unwrap_or_default()),
        "response.reasoning_summary_text.delta" => {
            Ok(match (payload.delta, payload.summary_index) {
                (Some(text), Some(summary_index)) => {
                    vec![ModelStreamEvent::ReasoningSummaryDelta {
                        text,
                        summary_index,
                    }]
                }
                _ => Vec::new(),
            })
        }
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            let call_id = payload
                .call_id
                .or(payload.item_id)
                .ok_or_else(|| ModelRuntimeError::StreamParse("missing call_id".to_string()))?;
            Ok(payload
                .delta
                .map(|delta| vec![ModelStreamEvent::ToolArgumentsDelta { call_id, delta }])
                .unwrap_or_default())
        }
        "response.output_item.added" => Ok(payload
            .item
            .and_then(|item| serde_json::from_value::<ResponseItem>(item).ok())
            .and_then(|item| tool_call_started_event(&item))
            .into_iter()
            .collect()),
        "response.output_item.done" => {
            let item = payload
                .item
                .ok_or_else(|| ModelRuntimeError::StreamParse("missing response item".to_string()))
                .and_then(|item| {
                    serde_json::from_value::<ResponseItem>(item)
                        .map_err(|err| ModelRuntimeError::StreamParse(err.to_string()))
                })?;

            let mut events = Vec::new();
            if let Some(tool_event) = tool_call_completed_event(&item) {
                events.push(tool_event);
            }
            if structured_output_requested && let Some(value) = structured_output_from_item(&item) {
                events.push(ModelStreamEvent::StructuredOutput { value });
            }
            events.push(ModelStreamEvent::ResponseItem(item));
            Ok(events)
        }
        "response.completed" => {
            let usage = payload
                .response
                .as_ref()
                .and_then(|response| response.get("usage"))
                .and_then(parse_usage);
            let mut events = Vec::new();
            if let Some(usage) = usage {
                events.push(ModelStreamEvent::Usage(usage));
            }
            events.push(ModelStreamEvent::Completed { stop_reason: None });
            Ok(events)
        }
        "response.failed" => {
            let message = payload
                .response
                .as_ref()
                .and_then(|response| response.get("error"))
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("response.failed");
            Err(ModelRuntimeError::Provider(message.to_string()))
        }
        "response.incomplete" => Err(ModelRuntimeError::Provider(
            "response.incomplete".to_string(),
        )),
        _ => Ok(Vec::new()),
    }
}

fn parse_usage(value: &Value) -> Option<TokenUsage> {
    Some(TokenUsage {
        input_tokens: value.get("input_tokens")?.as_i64()?,
        cached_input_tokens: value
            .get("input_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        output_tokens: value.get("output_tokens")?.as_i64()?,
        reasoning_output_tokens: value
            .get("output_tokens_details")
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        total_tokens: value.get("total_tokens")?.as_i64()?,
    })
}

fn tool_call_started_event(item: &ResponseItem) -> Option<ModelStreamEvent> {
    match item {
        ResponseItem::FunctionCall {
            call_id,
            name,
            namespace,
            ..
        } => Some(ModelStreamEvent::ToolCallStarted {
            call_id: call_id.clone(),
            name: name.clone(),
            namespace: namespace.clone(),
        }),
        ResponseItem::CustomToolCall { call_id, name, .. } => {
            Some(ModelStreamEvent::ToolCallStarted {
                call_id: call_id.clone(),
                name: name.clone(),
                namespace: None,
            })
        }
        _ => None,
    }
}

fn tool_call_completed_event(item: &ResponseItem) -> Option<ModelStreamEvent> {
    match item {
        ResponseItem::FunctionCall {
            call_id,
            name,
            namespace,
            ..
        } => Some(ModelStreamEvent::ToolCallCompleted {
            call_id: call_id.clone(),
            name: name.clone(),
            namespace: namespace.clone(),
        }),
        ResponseItem::CustomToolCall { call_id, name, .. } => {
            Some(ModelStreamEvent::ToolCallCompleted {
                call_id: call_id.clone(),
                name: name.clone(),
                namespace: None,
            })
        }
        _ => None,
    }
}

fn structured_output_from_item(item: &ResponseItem) -> Option<Value> {
    let ResponseItem::Message { content, .. } = item else {
        return None;
    };
    let text = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::OutputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    serde_json::from_str(&text).ok()
}

fn map_tool_spec(tool: &DynamicToolSpec) -> Value {
    serde_json::json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    })
}

async fn resolve_bearer_token(
    provider: &ModelProviderInfo,
) -> Result<Option<String>, ModelRuntimeError> {
    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(Some(token));
    }
    if let Some(token) = provider
        .api_key()
        .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?
    {
        return Ok(Some(token));
    }
    let Some(auth) = provider.auth.as_ref() else {
        return Ok(None);
    };

    let mut command = Command::new(&auth.command);
    command.args(&auth.args);
    command.current_dir(auth.cwd.as_path());

    let output = timeout(auth.timeout(), command.output())
        .await
        .map_err(|_| ModelRuntimeError::InvalidRequest("auth command timed out".to_string()))?
        .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;

    if !output.status.success() {
        return Err(ModelRuntimeError::InvalidRequest(format!(
            "auth command exited with status {}",
            output.status
        )));
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(ModelRuntimeError::InvalidRequest(
            "auth command returned an empty token".to_string(),
        ));
    }
    Ok(Some(token))
}

fn map_transport_error(error: TransportError) -> ModelRuntimeError {
    match error {
        TransportError::Build(message)
        | TransportError::Network(message)
        | TransportError::Http { body: message, .. } => ModelRuntimeError::Http(message),
        TransportError::Timeout => ModelRuntimeError::StreamInterrupted("idle timeout".to_string()),
        TransportError::RetryLimit => {
            ModelRuntimeError::StreamInterrupted("retry limit reached".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use bytes::Bytes;
    use futures::stream;
    use http::StatusCode;
    use model_runtime::transport::HttpResponse;
    use model_runtime::transport::StreamResponse;
    use model_runtime::transport::WsConnectResponse;
    use model_runtime::transport::WsMessageStream;
    use model_runtime::transport::WsResponse;
    use pretty_assertions::assert_eq;
    use runtime_protocol::models::ResponseInputItem;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    #[derive(Clone)]
    struct FixtureTransport {
        sse_body: String,
    }

    #[async_trait]
    impl HttpTransport for FixtureTransport {
        async fn execute(&self, _request: HttpRequest) -> Result<HttpResponse, TransportError> {
            Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from(
                    r#"{"data":[{"id":"demo","display_name":"Demo","context_window":4096,"max_context_window":4096,"supports_parallel_tool_calls":false,"supports_reasoning_summaries":false,"input_modalities":["text"]}]}"#,
                ),
            })
        }

        async fn stream(&self, _request: HttpRequest) -> Result<StreamResponse, TransportError> {
            Ok(StreamResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                bytes: Box::pin(stream::iter(vec![Ok(Bytes::from(self.sse_body.clone()))])),
            })
        }
    }

    struct SequencedTransport {
        sse_bodies: Mutex<VecDeque<String>>,
    }

    #[async_trait]
    impl HttpTransport for SequencedTransport {
        async fn execute(&self, _request: HttpRequest) -> Result<HttpResponse, TransportError> {
            unreachable!("execute not used")
        }

        async fn stream(&self, _request: HttpRequest) -> Result<StreamResponse, TransportError> {
            let next_body = self
                .sse_bodies
                .lock()
                .expect("sequence lock")
                .pop_front()
                .expect("missing queued SSE body");
            Ok(StreamResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                bytes: Box::pin(stream::iter(vec![Ok(Bytes::from(next_body))])),
            })
        }
    }

    #[derive(Default)]
    struct RecordingWebsocketConnection {
        sent: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl model_runtime::transport::WebsocketConnection for RecordingWebsocketConnection {
        async fn send_text(&mut self, text: String) -> Result<(), TransportError> {
            self.sent.lock().expect("sent lock").push(text);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FixtureWebsocketTransport {
        events: Arc<Mutex<VecDeque<WsMessage>>>,
        connect_count: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl WebsocketTransport for FixtureWebsocketTransport {
        async fn connect(&self, _request: WsRequest) -> Result<WsConnectResponse, TransportError> {
            *self.connect_count.lock().expect("connect count") += 1;
            let events = self
                .events
                .lock()
                .expect("events lock")
                .drain(..)
                .collect::<Vec<_>>();
            let messages: WsMessageStream = Box::pin(stream::iter(
                events.into_iter().map(Ok::<_, TransportError>),
            ));
            Ok(WsConnectResponse {
                connection: Box::new(RecordingWebsocketConnection::default()),
                response: WsResponse {
                    status: StatusCode::SWITCHING_PROTOCOLS,
                    headers: HeaderMap::new(),
                },
                messages,
            })
        }
    }

    #[test]
    fn request_body_maps_messages_and_tools() {
        let provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));
        let runtime = OpenAiCompatibleRuntime::new(provider.clone()).expect("runtime");
        let mut request = ModelTurnRequest::new(provider, "demo");
        request.messages.push(ResponseInputItem::Message {
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
            phase: None,
        });
        request.tools.push(DynamicToolSpec {
            namespace: None,
            name: "echo".to_string(),
            description: "Echo".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
            defer_loading: false,
        });
        request.allow_parallel_tool_calls = true;

        let body = runtime.build_request_body(&request, None, None);

        assert_eq!(body["model"], Value::String("demo".to_string()));
        assert_eq!(body["parallel_tool_calls"], Value::Bool(true));
        assert_eq!(body["input"][0]["role"], Value::String("user".to_string()));
    }

    #[tokio::test]
    async fn stream_turn_emits_text_and_completed() {
        let body = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
        );
        let mut provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));
        provider.supports_websockets = false;
        let runtime = OpenAiCompatibleRuntime::new_with_http_transport(
            provider.clone(),
            Arc::new(FixtureTransport {
                sse_body: body.to_string(),
            }),
        )
        .expect("runtime");
        let request = ModelTurnRequest::new(provider, "demo");

        let mut stream = runtime.stream_turn(request).await.expect("stream");
        let mut saw_text = false;
        let mut saw_completed = false;

        while let Some(event) = stream.next().await {
            match event.expect("model event") {
                ModelStreamEvent::TextDelta { text } => {
                    saw_text = true;
                    assert_eq!(text, "Hello");
                }
                ModelStreamEvent::Completed { .. } => saw_completed = true,
                _ => {}
            }
        }

        assert!(saw_text);
        assert!(saw_completed);
    }

    #[tokio::test]
    async fn stream_error_does_not_break_next_turn() {
        let interrupted_body = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n"
        );
        let completed_body = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Again\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-2\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
        );
        let mut provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));
        provider.supports_websockets = false;
        let runtime = OpenAiCompatibleRuntime::new_with_http_transport(
            provider.clone(),
            Arc::new(SequencedTransport {
                sse_bodies: Mutex::new(VecDeque::from([
                    interrupted_body.to_string(),
                    completed_body.to_string(),
                ])),
            }),
        )
        .expect("runtime");

        let mut first_stream = runtime
            .stream_turn(ModelTurnRequest::new(provider.clone(), "demo"))
            .await
            .expect("first stream");
        let mut saw_first_error = false;
        while let Some(event) = first_stream.next().await {
            if matches!(
                event,
                Err(ModelRuntimeError::StreamInterrupted(message))
                    if message == "stream closed before completion"
            ) {
                saw_first_error = true;
                break;
            }
        }
        assert!(saw_first_error);

        let mut second_stream = runtime
            .stream_turn(ModelTurnRequest::new(provider, "demo"))
            .await
            .expect("second stream");
        let mut saw_text = false;
        let mut saw_completed = false;
        while let Some(event) = second_stream.next().await {
            match event.expect("second model event") {
                ModelStreamEvent::TextDelta { text } => {
                    saw_text = true;
                    assert_eq!(text, "Again");
                }
                ModelStreamEvent::Completed { .. } => saw_completed = true,
                _ => {}
            }
        }

        assert!(saw_text);
        assert!(saw_completed);
    }

    #[tokio::test]
    async fn websocket_path_dispatches_when_provider_supports_it() {
        let mut provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));
        provider.supports_websockets = true;
        let websocket_transport = FixtureWebsocketTransport {
            events: Arc::new(Mutex::new(VecDeque::from([
                WsMessage::Text(
                    serde_json::json!({"type":"response.output_text.delta","delta":"Hello from ws"})
                        .to_string(),
                ),
                WsMessage::Text(
                    serde_json::json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}})
                        .to_string(),
                ),
            ]))),
            connect_count: Arc::new(Mutex::new(0)),
        };
        let runtime = OpenAiCompatibleRuntime::new_with_transports_and_capabilities(
            provider.clone(),
            Arc::new(FixtureTransport {
                sse_body: String::new(),
            }),
            Arc::new(websocket_transport.clone()),
            default_capabilities(),
        )
        .expect("runtime");

        let mut stream = runtime
            .stream_turn(ModelTurnRequest::new(provider, "demo"))
            .await
            .expect("ws stream");
        let mut saw_text = false;
        while let Some(event) = stream.next().await {
            match event.expect("ws event") {
                ModelStreamEvent::TextDelta { text } => {
                    saw_text = true;
                    assert_eq!(text, "Hello from ws");
                }
                ModelStreamEvent::Completed { .. } => break,
                _ => {}
            }
        }

        assert!(saw_text);
        assert_eq!(*websocket_transport.connect_count.lock().expect("count"), 1);
    }
}
