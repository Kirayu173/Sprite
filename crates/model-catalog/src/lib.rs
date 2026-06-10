use async_trait::async_trait;
use runtime_protocol::model_capabilities::ModelInfo as RemoteModelInfo;
use runtime_protocol::model_capabilities::ReasoningEffort;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub tool_calling: bool,
    pub parallel_tools: bool,
    pub image_input: bool,
    pub reasoning: bool,
    pub reasoning_summaries: bool,
    pub structured_output: bool,
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub provider_id: String,
    pub capabilities: ModelCapabilities,
    pub default_reasoning_effort: Option<ReasoningEffort>,
    pub supported_reasoning_efforts: Vec<ReasoningEffort>,
}

impl ModelCatalogEntry {
    pub fn from_remote(provider_id: impl Into<String>, model: RemoteModelInfo) -> Self {
        let supported_reasoning_efforts = model
            .supported_reasoning_levels
            .iter()
            .map(|preset| preset.effort.clone())
            .collect::<Vec<_>>();

        Self {
            id: model.slug,
            display_name: model.display_name,
            provider_id: provider_id.into(),
            capabilities: ModelCapabilities {
                tool_calling: true,
                parallel_tools: model.supports_parallel_tool_calls,
                image_input: model.input_modalities.iter().any(|modality| {
                    matches!(
                        modality,
                        runtime_protocol::model_capabilities::InputModality::Image
                    )
                }),
                reasoning: model.default_reasoning_level.is_some()
                    || !supported_reasoning_efforts.is_empty(),
                reasoning_summaries: model.supports_reasoning_summaries,
                structured_output: true,
                context_window: model.context_window,
                max_context_window: model.max_context_window,
            },
            default_reasoning_effort: model.default_reasoning_level,
            supported_reasoning_efforts,
        }
    }
}

#[derive(Debug, Error)]
pub enum ModelCatalogError {
    #[error("catalog is not supported for provider `{0}`")]
    Unsupported(String),
    #[error("catalog request failed: {0}")]
    Http(String),
    #[error("catalog payload could not be parsed: {0}")]
    Parse(String),
}

#[async_trait]
pub trait ModelCatalog: Send + Sync {
    async fn list_models(&self) -> Result<Vec<ModelCatalogEntry>, ModelCatalogError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureCatalog;

    #[async_trait]
    impl ModelCatalog for FixtureCatalog {
        async fn list_models(&self) -> Result<Vec<ModelCatalogEntry>, ModelCatalogError> {
            Ok(vec![ModelCatalogEntry {
                id: "demo".to_string(),
                display_name: "Demo".to_string(),
                provider_id: "fixture".to_string(),
                capabilities: ModelCapabilities {
                    tool_calling: true,
                    parallel_tools: false,
                    image_input: false,
                    reasoning: false,
                    reasoning_summaries: false,
                    structured_output: true,
                    context_window: Some(4096),
                    max_context_window: Some(4096),
                },
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
            }])
        }
    }

    #[tokio::test]
    async fn fixture_catalog_lists_entries() {
        let catalog = FixtureCatalog;
        let entries = catalog.list_models().await.expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "demo");
        assert_eq!(entries[0].capabilities.context_window, Some(4096));
    }
}
