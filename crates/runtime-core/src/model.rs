use model_provider::ModelBackend;
use model_provider::ModelProviderError;
use model_provider::create_model_backend;
use model_provider_info::ModelProviderInfo;
use model_runtime::ModelCatalogEntry;
use model_runtime::ModelEventStream;
use model_runtime::ModelRuntimeError;
use model_runtime::ModelTurnRequest;
use model_runtime::ProviderCapabilities;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelBridgeError {
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
    #[error(transparent)]
    Runtime(#[from] ModelRuntimeError),
    #[error("model catalog error: {0}")]
    Catalog(String),
}

pub fn create_backend(
    provider_id: &str,
    provider: ModelProviderInfo,
) -> Result<ModelBackend, ModelProviderError> {
    create_model_backend(provider_id, provider)
}

pub fn provider_capabilities(
    provider_id: &str,
    provider: ModelProviderInfo,
) -> Result<ProviderCapabilities, ModelBridgeError> {
    let backend = create_backend(provider_id, provider)?;
    Ok(backend.runtime.provider_capabilities())
}

pub async fn list_models(
    provider_id: &str,
    provider: ModelProviderInfo,
) -> Result<Vec<ModelCatalogEntry>, ModelBridgeError> {
    let backend = create_backend(provider_id, provider)?;
    backend
        .catalog
        .list_models()
        .await
        .map_err(|err| ModelBridgeError::Catalog(err.to_string()))
}

pub async fn stream_turn(
    provider_id: &str,
    provider: ModelProviderInfo,
    request: ModelTurnRequest,
) -> Result<ModelEventStream, ModelBridgeError> {
    let backend = create_backend(provider_id, provider)?;
    backend
        .runtime
        .stream_turn(request)
        .await
        .map_err(Into::into)
}
