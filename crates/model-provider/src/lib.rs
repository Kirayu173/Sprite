use model_catalog::ModelCatalog;
use model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use model_provider_info::ModelProviderInfo;
use model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use model_runtime::ModelRuntime;
use provider_anthropic_compatible::AnthropicCompatibleRuntime;
use provider_lmstudio::LMStudioCatalog;
use provider_lmstudio::LMStudioRuntime;
use provider_ollama::OllamaCatalog;
use provider_ollama::OllamaRuntime;
use provider_openai_compatible::OpenAiCompatibleCatalog;
use provider_openai_compatible::OpenAiCompatibleRuntime;
use std::sync::Arc;
use thiserror::Error;

pub struct ModelBackend {
    pub runtime: Arc<dyn ModelRuntime>,
    pub catalog: Arc<dyn ModelCatalog>,
}

#[derive(Debug, Error)]
pub enum ModelProviderError {
    #[error("{0}")]
    Build(String),
}

pub fn create_model_backend(
    provider_id: &str,
    provider: ModelProviderInfo,
) -> Result<ModelBackend, ModelProviderError> {
    match provider_id {
        OLLAMA_OSS_PROVIDER_ID => Ok(ModelBackend {
            runtime: Arc::new(
                OllamaRuntime::new(provider.clone())
                    .map_err(|err| ModelProviderError::Build(err.to_string()))?,
            ),
            catalog: Arc::new(OllamaCatalog::new(provider)),
        }),
        LMSTUDIO_OSS_PROVIDER_ID => Ok(ModelBackend {
            runtime: Arc::new(
                LMStudioRuntime::new(provider.clone())
                    .map_err(|err| ModelProviderError::Build(err.to_string()))?,
            ),
            catalog: Arc::new(LMStudioCatalog::new(provider)),
        }),
        _ if provider.is_anthropic_compatible() => Ok(ModelBackend {
            runtime: Arc::new(
                AnthropicCompatibleRuntime::new(provider.clone())
                    .map_err(|err| ModelProviderError::Build(err.to_string()))?,
            ),
            catalog: Arc::new(OpenAiCompatibleCatalog::new(provider_id, provider)),
        }),
        _ => Ok(ModelBackend {
            runtime: Arc::new(
                OpenAiCompatibleRuntime::new(provider.clone())
                    .map_err(|err| ModelProviderError::Build(err.to_string()))?,
            ),
            catalog: Arc::new(OpenAiCompatibleCatalog::new(provider_id, provider)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model_provider_info::OPENAI_COMPATIBLE_PROVIDER_ID;

    #[test]
    fn create_model_backend_maps_builtin_provider_ids() {
        let openai_provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));
        let ollama_provider = model_provider_info::create_local_provider_with_base_url(
            "Ollama",
            "http://localhost:11434/v1",
            model_provider_info::WireApi::Responses,
        );
        let lmstudio_provider = model_provider_info::create_local_provider_with_base_url(
            "LM Studio",
            "http://localhost:1234/v1",
            model_provider_info::WireApi::Responses,
        );

        assert!(create_model_backend(OPENAI_COMPATIBLE_PROVIDER_ID, openai_provider).is_ok());
        assert!(
            create_model_backend(model_provider_info::OLLAMA_OSS_PROVIDER_ID, ollama_provider)
                .is_ok()
        );
        assert!(
            create_model_backend(
                model_provider_info::LMSTUDIO_OSS_PROVIDER_ID,
                lmstudio_provider
            )
            .is_ok()
        );
    }
}
