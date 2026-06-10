use async_trait::async_trait;
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
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;

pub use model_provider_info::DEFAULT_LMSTUDIO_MODEL;

#[derive(Clone)]
pub struct LMStudioRuntime {
    inner: OpenAiCompatibleRuntime,
}

impl LMStudioRuntime {
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

    pub async fn probe(provider: &ModelProviderInfo) -> Result<(), ModelRuntimeError> {
        LMStudioInstaller::new(provider.clone())?.probe().await
    }
}

#[async_trait]
impl ModelRuntime for LMStudioRuntime {
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
pub struct LMStudioCatalog {
    inner: OpenAiCompatibleCatalog,
}

impl LMStudioCatalog {
    pub fn new(provider: ModelProviderInfo) -> Self {
        Self {
            inner: OpenAiCompatibleCatalog::new("lmstudio", provider),
        }
    }
}

#[async_trait]
impl ModelCatalog for LMStudioCatalog {
    async fn list_models(&self) -> Result<Vec<ModelCatalogEntry>, ModelCatalogError> {
        self.inner.list_models().await
    }
}

#[async_trait]
pub trait ModelDownloader: Send + Sync {
    async fn download_model(&self, model: &str) -> Result<(), ModelRuntimeError>;
}

#[derive(Clone, Default)]
pub struct LmStudioCliDownloader;

#[async_trait]
impl ModelDownloader for LmStudioCliDownloader {
    async fn download_model(&self, model: &str) -> Result<(), ModelRuntimeError> {
        let lms = find_lms()?;
        let status = Command::new(&lms)
            .args(["get", "--yes", model])
            .status()
            .await
            .map_err(|err| ModelRuntimeError::Provider(err.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(ModelRuntimeError::Provider(format!(
                "LM Studio model download failed with status {status}"
            )))
        }
    }
}

#[derive(Clone)]
pub struct LMStudioInstaller {
    provider: ModelProviderInfo,
    transport: Arc<dyn HttpTransport>,
    downloader: Arc<dyn ModelDownloader>,
}

impl LMStudioInstaller {
    pub fn new(provider: ModelProviderInfo) -> Result<Self, ModelRuntimeError> {
        Self::new_with_dependencies(
            provider,
            Arc::new(ReqwestTransport::new()),
            Arc::new(LmStudioCliDownloader),
        )
    }

    pub fn new_with_dependencies(
        provider: ModelProviderInfo,
        transport: Arc<dyn HttpTransport>,
        downloader: Arc<dyn ModelDownloader>,
    ) -> Result<Self, ModelRuntimeError> {
        if provider.base_url.is_none() {
            return Err(ModelRuntimeError::InvalidRequest(
                "provider.base_url is required".to_string(),
            ));
        }
        Ok(Self {
            provider,
            transport,
            downloader,
        })
    }

    pub async fn probe(&self) -> Result<(), ModelRuntimeError> {
        self.fetch_models().await.map(|_| ())
    }

    pub async fn ensure_oss_ready(&self, model: &str) -> Result<(), ModelRuntimeError> {
        let models = self.fetch_models().await?;
        if !models.iter().any(|existing| existing == model) {
            self.downloader.download_model(model).await?;
        }
        self.load_model(model).await
    }

    pub async fn fetch_models(&self) -> Result<Vec<String>, ModelRuntimeError> {
        let response = self
            .transport
            .execute(HttpRequest::get(
                self.base_url("/models")?,
                HeaderMap::new(),
            ))
            .await
            .map_err(map_transport_error)?;
        let payload: Value = serde_json::from_slice(&response.body)
            .map_err(|err| ModelRuntimeError::StreamParse(err.to_string()))?;
        let data = payload
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ModelRuntimeError::StreamParse("missing model data array".to_string())
            })?;
        Ok(data
            .iter()
            .filter_map(|entry| entry.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect())
    }

    pub async fn load_model(&self, model: &str) -> Result<(), ModelRuntimeError> {
        self.transport
            .execute(HttpRequest::post_json(
                self.base_url("/responses")?,
                HeaderMap::new(),
                serde_json::json!({
                    "model": model,
                    "input": "",
                    "max_output_tokens": 1,
                    "stream": false,
                }),
            ))
            .await
            .map(|_| ())
            .map_err(map_transport_error)
    }

    fn base_url(&self, path: &str) -> Result<String, ModelRuntimeError> {
        let base_url = self.provider.base_url.as_deref().ok_or_else(|| {
            ModelRuntimeError::InvalidRequest("provider.base_url is required".to_string())
        })?;
        Ok(format!("{}{}", base_url.trim_end_matches('/'), path))
    }
}

fn find_lms() -> Result<String, ModelRuntimeError> {
    if which::which("lms").is_ok() {
        return Ok("lms".to_string());
    }

    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").unwrap_or_default()
    } else {
        std::env::var("HOME").unwrap_or_default()
    };

    #[cfg(unix)]
    let fallback_path = format!("{home}/.lmstudio/bin/lms");

    #[cfg(windows)]
    let fallback_path = format!("{home}\\.lmstudio\\bin\\lms.exe");

    if Path::new(&fallback_path).exists() {
        Ok(fallback_path)
    } else {
        Err(ModelRuntimeError::Provider(
            "LM Studio not found. Install it from https://lmstudio.ai/.".to_string(),
        ))
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
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl FixtureTransport {
        fn new(responses: Vec<Result<HttpResponse, TransportError>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
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

        async fn stream(&self, _request: HttpRequest) -> Result<StreamResponse, TransportError> {
            unreachable!("stream not used")
        }
    }

    #[derive(Default)]
    struct RecordingDownloader {
        downloads: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ModelDownloader for RecordingDownloader {
        async fn download_model(&self, model: &str) -> Result<(), ModelRuntimeError> {
            self.downloads
                .lock()
                .expect("downloads")
                .push(model.to_string());
            Ok(())
        }
    }

    fn provider() -> ModelProviderInfo {
        model_provider_info::create_local_provider_with_base_url(
            "LM Studio",
            "http://localhost:1234/v1",
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

    #[tokio::test]
    async fn ensure_oss_ready_downloads_when_missing() {
        let transport = Arc::new(FixtureTransport::new(vec![
            json_response(serde_json::json!({
                "data": [{"id": "other"}]
            })),
            json_response(serde_json::json!({
                "id": "load-1"
            })),
        ]));
        let downloader = Arc::new(RecordingDownloader::default());
        let installer = LMStudioInstaller::new_with_dependencies(
            provider(),
            transport.clone(),
            downloader.clone(),
        )
        .expect("installer");

        installer
            .ensure_oss_ready(DEFAULT_LMSTUDIO_MODEL)
            .await
            .expect("ensure");

        assert_eq!(
            downloader.downloads.lock().expect("downloads").as_slice(),
            [DEFAULT_LMSTUDIO_MODEL]
        );
        let requests = transport.requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, Method::GET);
        assert_eq!(requests[1].method, Method::POST);
        assert!(requests[1].url.ends_with("/responses"));
    }

    #[tokio::test]
    async fn ensure_oss_ready_skips_download_when_present() {
        let transport = Arc::new(FixtureTransport::new(vec![
            json_response(serde_json::json!({
                "data": [{"id": DEFAULT_LMSTUDIO_MODEL}]
            })),
            json_response(serde_json::json!({
                "id": "load-1"
            })),
        ]));
        let downloader = Arc::new(RecordingDownloader::default());
        let installer =
            LMStudioInstaller::new_with_dependencies(provider(), transport, downloader.clone())
                .expect("installer");

        installer
            .ensure_oss_ready(DEFAULT_LMSTUDIO_MODEL)
            .await
            .expect("ensure");

        assert!(downloader.downloads.lock().expect("downloads").is_empty());
    }
}
