use model_catalog::ModelCapabilities;
use model_catalog::ModelCatalogEntry;
use runtime_protocol::model_capabilities::InputModality;
use runtime_protocol::model_capabilities::ModelInfo;
use runtime_protocol::model_capabilities::ReasoningEffort;
use runtime_protocol::model_capabilities::ReasoningEffortPreset;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct RemoteModelsResponse {
    #[serde(default)]
    pub data: Vec<RemoteModelRecord>,
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

impl RemoteModelsResponse {
    pub fn into_catalog_entries(self, provider_id: impl Into<String>) -> Vec<ModelCatalogEntry> {
        let provider_id = provider_id.into();
        if !self.models.is_empty() {
            return self
                .models
                .into_iter()
                .map(|model| ModelCatalogEntry::from_remote(provider_id.clone(), model))
                .collect();
        }

        self.data
            .into_iter()
            .map(|model| model.into_catalog_entry(provider_id.clone()))
            .collect()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct RemoteModelRecord {
    pub id: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub context_window: Option<i64>,
    #[serde(default)]
    pub max_context_window: Option<i64>,
    #[serde(default)]
    pub supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub supports_reasoning_summaries: bool,
    #[serde(default)]
    pub input_modalities: Vec<InputModality>,
    #[serde(default)]
    pub default_reasoning_level: Option<ReasoningEffort>,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<ReasoningEffortPreset>,
}

impl RemoteModelRecord {
    fn into_catalog_entry(self, provider_id: String) -> ModelCatalogEntry {
        let supported_reasoning_efforts = self
            .supported_reasoning_levels
            .iter()
            .map(|preset| preset.effort.clone())
            .collect::<Vec<_>>();
        let id = self.slug.unwrap_or(self.id);
        let display_name = self.display_name.clone().unwrap_or_else(|| id.clone());

        ModelCatalogEntry {
            id,
            display_name,
            provider_id,
            capabilities: ModelCapabilities {
                tool_calling: true,
                parallel_tools: self.supports_parallel_tool_calls,
                image_input: self
                    .input_modalities
                    .iter()
                    .any(|modality| matches!(modality, InputModality::Image)),
                reasoning: self.default_reasoning_level.is_some()
                    || !supported_reasoning_efforts.is_empty(),
                reasoning_summaries: self.supports_reasoning_summaries,
                structured_output: true,
                context_window: self.context_window,
                max_context_window: self.max_context_window,
            },
            default_reasoning_effort: self.default_reasoning_level,
            supported_reasoning_efforts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_models_payload_maps_to_catalog_entry() {
        let response: RemoteModelsResponse = serde_json::from_value(serde_json::json!({
            "data": [{
                "id": "gpt-oss",
                "display_name": "GPT OSS",
                "context_window": 8192,
                "max_context_window": 16384,
                "supports_parallel_tool_calls": true,
                "supports_reasoning_summaries": true,
                "input_modalities": ["text", "image"]
            }]
        }))
        .expect("remote models");

        let entries = response.into_catalog_entries("openai-compatible");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "gpt-oss");
        assert_eq!(entries[0].display_name, "GPT OSS");
        assert!(entries[0].capabilities.parallel_tools);
        assert!(entries[0].capabilities.image_input);
    }
}
