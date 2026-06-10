use model_provider_info::ModelProviderInfo;
use runtime_protocol::config_types::Personality;
use runtime_protocol::config_types::ReasoningSummary;
use runtime_protocol::config_types::Verbosity;
use runtime_protocol::dynamic_tools::DynamicToolSpec;
use runtime_protocol::model_capabilities::ReasoningEffort;
use runtime_protocol::models::BaseInstructions;
use runtime_protocol::models::ResponseInputItem;
use runtime_protocol::protocol::InterAgentCommunication;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::provider::ProviderCapabilities;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelTurnRequest {
    pub provider: ModelProviderInfo,
    pub model: String,
    pub messages: Vec<ResponseInputItem>,
    pub tools: Vec<DynamicToolSpec>,
    pub base_instructions: BaseInstructions,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_summary: Option<ReasoningSummary>,
    pub verbosity: Option<Verbosity>,
    pub max_output_tokens: Option<u32>,
    pub output_schema: Option<Value>,
    pub output_schema_strict: bool,
    pub allow_parallel_tool_calls: bool,
    pub selected_capabilities: Option<ProviderCapabilities>,
    pub previous_response_id: Option<String>,
    pub metadata: HashMap<String, String>,
    pub personality: Option<Personality>,
    pub inter_agent_communication: Option<InterAgentCommunication>,
}

impl ModelTurnRequest {
    pub fn new(provider: ModelProviderInfo, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            base_instructions: BaseInstructions::default(),
            reasoning_effort: None,
            reasoning_summary: None,
            verbosity: None,
            max_output_tokens: None,
            output_schema: None,
            output_schema_strict: true,
            allow_parallel_tool_calls: false,
            selected_capabilities: None,
            previous_response_id: None,
            metadata: HashMap::new(),
            personality: None,
            inter_agent_communication: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model_provider_info::ModelProviderInfo;

    #[test]
    fn new_request_starts_with_runtime_safe_defaults() {
        let provider = ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://example.test/v1".to_string(),
        ));

        let request = ModelTurnRequest::new(provider.clone(), "demo");

        assert_eq!(request.provider, provider);
        assert_eq!(request.model, "demo");
        assert!(request.messages.is_empty());
        assert!(request.tools.is_empty());
        assert_eq!(request.base_instructions, BaseInstructions::default());
        assert_eq!(request.reasoning_effort, None);
        assert_eq!(request.reasoning_summary, None);
        assert_eq!(request.verbosity, None);
        assert_eq!(request.max_output_tokens, None);
        assert_eq!(request.output_schema, None);
        assert!(request.output_schema_strict);
        assert!(!request.allow_parallel_tool_calls);
        assert_eq!(request.selected_capabilities, None);
        assert_eq!(request.previous_response_id, None);
        assert!(request.metadata.is_empty());
        assert_eq!(request.personality, None);
        assert_eq!(request.inter_agent_communication, None);
    }
}
