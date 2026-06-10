use async_trait::async_trait;
use http::header::AUTHORIZATION;
use model_catalog::ModelCatalog;
use model_catalog::ModelCatalogEntry;
use model_catalog::ModelCatalogError;
use model_provider_info::ModelProviderInfo;
use model_runtime::transport::HttpRequest;
use model_runtime::transport::HttpTransport;
use model_runtime::transport::ReqwestTransport;
use std::sync::Arc;

use crate::models::RemoteModelsResponse;
use crate::runtime::OpenAiCompatibleRuntime;

#[derive(Clone)]
pub struct OpenAiCompatibleCatalog {
    provider_id: String,
    provider: ModelProviderInfo,
    transport: Arc<dyn HttpTransport>,
}

impl OpenAiCompatibleCatalog {
    pub fn new(provider_id: impl Into<String>, provider: ModelProviderInfo) -> Self {
        Self::new_with_transport(provider_id, provider, Arc::new(ReqwestTransport::new()))
    }

    pub fn new_with_transport(
        provider_id: impl Into<String>,
        provider: ModelProviderInfo,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            provider,
            transport,
        }
    }
}

#[async_trait]
impl ModelCatalog for OpenAiCompatibleCatalog {
    async fn list_models(&self) -> Result<Vec<ModelCatalogEntry>, ModelCatalogError> {
        let runtime = OpenAiCompatibleRuntime::new_with_http_transport(
            self.provider.clone(),
            self.transport.clone(),
        )
        .map_err(|err| ModelCatalogError::Unsupported(err.to_string()))?;

        let endpoint = runtime
            .endpoint_url("models")
            .map_err(|err| ModelCatalogError::Http(err.to_string()))?;
        let mut headers = runtime
            .base_headers()
            .map_err(|err| ModelCatalogError::Http(err.to_string()))?;
        if let Some(auth_header) = runtime
            .auth_header()
            .await
            .map_err(|err| ModelCatalogError::Http(err.to_string()))?
        {
            headers.insert(AUTHORIZATION, auth_header);
        }

        let response = self
            .transport
            .execute(HttpRequest::get(endpoint, headers))
            .await
            .map_err(|err| ModelCatalogError::Http(err.to_string()))?;

        let body: RemoteModelsResponse = serde_json::from_slice(&response.body)
            .map_err(|err| ModelCatalogError::Parse(err.to_string()))?;

        Ok(body.into_catalog_entries(self.provider_id.clone()))
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use bytes::Bytes;
    use http::HeaderMap;
    use http::StatusCode;
    use model_runtime::transport::HttpResponse;
    use model_runtime::transport::StreamResponse;
    use model_runtime::transport::TransportError;
    use std::sync::Arc;

    use super::*;

    #[derive(Clone)]
    struct FixtureTransport;

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
            unreachable!("stream not used in catalog test")
        }
    }

    #[tokio::test]
    async fn catalog_lists_remote_models() {
        let provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));
        let catalog = OpenAiCompatibleCatalog::new_with_transport(
            "openai-compatible",
            provider,
            Arc::new(FixtureTransport),
        );

        let models = catalog.list_models().await.expect("models");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "demo");
        assert_eq!(models[0].capabilities.context_window, Some(4096));
    }
}
