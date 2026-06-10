use async_trait::async_trait;
use futures::StreamExt;
use http::HeaderMap;
use model_catalog::ModelCatalog;
use model_catalog::ModelCatalogEntry;
use model_catalog::ModelCatalogError;
use model_provider_info::ModelProviderInfo;
use model_runtime::ModelEventStream;
use model_runtime::ModelRuntime;
use model_runtime::ModelRuntimeError;
use model_runtime::ModelTurnRequest;
use model_runtime::ProviderCapabilities;
use model_runtime::transport::HttpRequest;
use model_runtime::transport::HttpTransport;
use model_runtime::transport::ReqwestTransport;
use provider_openai_compatible::OpenAiCompatibleCatalog;
use provider_openai_compatible::OpenAiCompatibleRuntime;
use serde_json::Value;
use std::sync::Arc;

pub use model_provider_info::DEFAULT_OLLAMA_MODEL;

#[derive(Clone)]
pub struct OllamaRuntime {
    inner: OpenAiCompatibleRuntime,
}

impl OllamaRuntime {
    pub fn new(provider: ModelProviderInfo) -> Result<Self, ModelRuntimeError> {
        Ok(Self {
            inner: OpenAiCompatibleRuntime::new_with_transport_and_capabilities(
                provider,
                Arc::new(ReqwestTransport::new()),
                ProviderCapabilities {
                    tool_calling: true,
                    parallel_tools: false,
                    image_input: true,
                    reasoning: false,
                    reasoning_summaries: false,
                    structured_output: true,
                    context_window: None,
                },
            )?,
        })
    }

    pub async fn probe(provider: &ModelProviderInfo) -> Result<OllamaProbe, ModelRuntimeError> {
        OllamaInstaller::new(provider.clone())?.probe().await
    }
}

#[async_trait]
impl ModelRuntime for OllamaRuntime {
    fn provider_capabilities(&self) -> ProviderCapabilities {
        self.inner.provider_capabilities()
    }

    async fn stream_turn(
        &self,
        request: ModelTurnRequest,
    ) -> Result<ModelEventStream, ModelRuntimeError> {
        self.inner.stream_turn(request).await
    }
}

#[derive(Clone)]
pub struct OllamaCatalog {
    inner: OpenAiCompatibleCatalog,
}

impl OllamaCatalog {
    pub fn new(provider: ModelProviderInfo) -> Self {
        Self {
            inner: OpenAiCompatibleCatalog::new("ollama", provider),
        }
    }
}

#[async_trait]
impl ModelCatalog for OllamaCatalog {
    async fn list_models(&self) -> Result<Vec<ModelCatalogEntry>, ModelCatalogError> {
        self.inner.list_models().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaProbe {
    pub version: Option<String>,
}

#[derive(Clone)]
pub struct OllamaInstaller {
    provider: ModelProviderInfo,
    transport: Arc<dyn HttpTransport>,
}

impl OllamaInstaller {
    pub fn new(provider: ModelProviderInfo) -> Result<Self, ModelRuntimeError> {
        Self::new_with_transport(provider, Arc::new(ReqwestTransport::new()))
    }

    pub fn new_with_transport(
        provider: ModelProviderInfo,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, ModelRuntimeError> {
        if provider.base_url.is_none() {
            return Err(ModelRuntimeError::InvalidRequest(
                "provider.base_url is required".to_string(),
            ));
        }
        Ok(Self {
            provider,
            transport,
        })
    }

    pub async fn probe(&self) -> Result<OllamaProbe, ModelRuntimeError> {
        self.fetch_models().await?;
        let version = self.fetch_version().await?;
        Ok(OllamaProbe { version })
    }

    pub async fn ensure_oss_ready(&self, model: &str) -> Result<(), ModelRuntimeError> {
        let models = self.fetch_models().await?;
        if models.iter().any(|existing| existing == model) {
            return Ok(());
        }
        self.pull_model(model).await
    }

    pub async fn fetch_models(&self) -> Result<Vec<String>, ModelRuntimeError> {
        let response = self
            .transport
            .execute(HttpRequest::get(
                self.host_url("/api/tags")?,
                HeaderMap::new(),
            ))
            .await
            .map_err(map_transport_error)?;
        let payload: Value = serde_json::from_slice(&response.body)
            .map_err(|err| ModelRuntimeError::StreamParse(err.to_string()))?;
        Ok(payload
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| model.get("name").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect())
    }

    pub async fn fetch_version(&self) -> Result<Option<String>, ModelRuntimeError> {
        let response = self
            .transport
            .execute(HttpRequest::get(
                self.host_url("/api/version")?,
                HeaderMap::new(),
            ))
            .await;

        match response {
            Ok(response) => {
                let payload: Value = serde_json::from_slice(&response.body)
                    .map_err(|err| ModelRuntimeError::StreamParse(err.to_string()))?;
                Ok(payload
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string))
            }
            Err(model_runtime::transport::TransportError::Http { status, .. })
                if status == http::StatusCode::NOT_FOUND =>
            {
                Ok(None)
            }
            Err(error) => Err(map_transport_error(error)),
        }
    }

    async fn pull_model(&self, model: &str) -> Result<(), ModelRuntimeError> {
        let response = self
            .transport
            .stream(HttpRequest::post_json(
                self.host_url("/api/pull")?,
                HeaderMap::new(),
                serde_json::json!({
                    "model": model,
                    "stream": true,
                }),
            ))
            .await
            .map_err(map_transport_error)?;

        let mut bytes = response.bytes;
        let mut buffer = String::new();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(map_transport_error)?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(newline_index) = buffer.find('\n') {
                let line = buffer[..newline_index].trim().to_string();
                buffer.drain(..=newline_index);
                if line.is_empty() {
                    continue;
                }
                let payload: Value = serde_json::from_str(&line)
                    .map_err(|err| ModelRuntimeError::StreamParse(err.to_string()))?;
                if let Some(error) = payload.get("error").and_then(Value::as_str) {
                    return Err(ModelRuntimeError::Provider(error.to_string()));
                }
                if payload
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status.eq_ignore_ascii_case("success"))
                {
                    return Ok(());
                }
            }
        }

        Err(ModelRuntimeError::StreamInterrupted(
            "ollama pull stream ended before reporting success".to_string(),
        ))
    }

    fn host_url(&self, path: &str) -> Result<String, ModelRuntimeError> {
        let base_url = self.provider.base_url.as_deref().ok_or_else(|| {
            ModelRuntimeError::InvalidRequest("provider.base_url is required".to_string())
        })?;
        let host_root = base_url.trim_end_matches("/v1").trim_end_matches('/');
        Ok(format!("{host_root}{path}"))
    }
}

fn map_transport_error(error: model_runtime::transport::TransportError) -> ModelRuntimeError {
    match error {
        model_runtime::transport::TransportError::Build(message)
        | model_runtime::transport::TransportError::Network(message)
        | model_runtime::transport::TransportError::Http { body: message, .. } => {
            ModelRuntimeError::Http(message)
        }
        model_runtime::transport::TransportError::Timeout => {
            ModelRuntimeError::StreamInterrupted("idle timeout".to_string())
        }
        model_runtime::transport::TransportError::RetryLimit => {
            ModelRuntimeError::StreamInterrupted("retry limit reached".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use bytes::Bytes;
    use futures::stream;
    use http::Method;
    use http::StatusCode;
    use model_runtime::transport::HttpResponse;
    use model_runtime::transport::StreamResponse;
    use model_runtime::transport::TransportError;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    struct FixtureTransport {
        responses: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
        streams: Mutex<VecDeque<Result<StreamResponse, TransportError>>>,
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl FixtureTransport {
        fn new(
            responses: Vec<Result<HttpResponse, TransportError>>,
            streams: Vec<Result<StreamResponse, TransportError>>,
        ) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
                streams: Mutex::new(VecDeque::from(streams)),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HttpTransport for FixtureTransport {
        async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.requests.lock().expect("requests").push(request);
            self.responses
                .lock()
                .expect("responses")
                .pop_front()
                .expect("queued response")
        }

        async fn stream(&self, request: HttpRequest) -> Result<StreamResponse, TransportError> {
            self.requests.lock().expect("requests").push(request);
            self.streams
                .lock()
                .expect("streams")
                .pop_front()
                .expect("queued stream")
        }
    }

    fn provider() -> ModelProviderInfo {
        model_provider_info::create_local_provider_with_base_url(
            "Ollama",
            "http://localhost:11434/v1",
            model_provider_info::WireApi::Responses,
        )
    }

    fn json_response(body: serde_json::Value) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from(body.to_string()),
        })
    }

    fn json_stream(lines: &[serde_json::Value]) -> Result<StreamResponse, TransportError> {
        let body = lines
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream::iter(vec![Ok(Bytes::from(format!("{body}\n")))])),
        })
    }

    #[tokio::test]
    async fn probe_reports_ollama_version() {
        let transport = Arc::new(FixtureTransport::new(
            vec![
                json_response(serde_json::json!({ "models": [] })),
                json_response(serde_json::json!({ "version": "0.14.1" })),
            ],
            vec![],
        ));
        let installer =
            OllamaInstaller::new_with_transport(provider(), transport).expect("installer");

        let probe = installer.probe().await.expect("probe");

        assert_eq!(
            probe,
            OllamaProbe {
                version: Some("0.14.1".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn ensure_oss_ready_pulls_when_missing() {
        let transport = Arc::new(FixtureTransport::new(
            vec![json_response(serde_json::json!({
                "models": [{"name": "other"}]
            }))],
            vec![json_stream(&[
                serde_json::json!({ "status": "pulling manifest" }),
                serde_json::json!({ "status": "success" }),
            ])],
        ));
        let installer =
            OllamaInstaller::new_with_transport(provider(), transport.clone()).expect("installer");

        installer
            .ensure_oss_ready(DEFAULT_OLLAMA_MODEL)
            .await
            .expect("ensure");

        let requests = transport.requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, Method::GET);
        assert_eq!(requests[1].method, Method::POST);
        assert!(requests[1].url.ends_with("/api/pull"));
    }

    #[tokio::test]
    async fn ensure_oss_ready_skips_pull_when_model_exists() {
        let transport = Arc::new(FixtureTransport::new(
            vec![json_response(serde_json::json!({
                "models": [{"name": DEFAULT_OLLAMA_MODEL}]
            }))],
            vec![],
        ));
        let installer =
            OllamaInstaller::new_with_transport(provider(), transport.clone()).expect("installer");

        installer
            .ensure_oss_ready(DEFAULT_OLLAMA_MODEL)
            .await
            .expect("ensure");

        let requests = transport.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
    }
}
