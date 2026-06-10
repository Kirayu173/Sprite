use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::event::ModelEventStream;
use crate::request::ModelTurnRequest;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub tool_calling: bool,
    pub parallel_tools: bool,
    pub image_input: bool,
    pub reasoning: bool,
    pub reasoning_summaries: bool,
    pub structured_output: bool,
    pub context_window: Option<i64>,
}

#[derive(Debug, Error)]
pub enum ModelRuntimeError {
    #[error("invalid model runtime request: {0}")]
    InvalidRequest(String),
    #[error("model runtime is not configured for this provider: {0}")]
    UnsupportedProvider(String),
    #[error("http transport error: {0}")]
    Http(String),
    #[error("stream parse error: {0}")]
    StreamParse(String),
    #[error("provider returned an error response: {0}")]
    Provider(String),
    #[error("stream interrupted before completion: {0}")]
    StreamInterrupted(String),
}

#[async_trait]
pub trait ModelRuntime: Send + Sync {
    fn provider_capabilities(&self) -> ProviderCapabilities;

    async fn stream_turn(
        &self,
        request: ModelTurnRequest,
    ) -> Result<ModelEventStream, ModelRuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_capabilities_default_to_disabled_flags() {
        let capabilities = ProviderCapabilities::default();

        assert!(!capabilities.tool_calling);
        assert!(!capabilities.parallel_tools);
        assert!(!capabilities.image_input);
        assert!(!capabilities.reasoning);
        assert!(!capabilities.reasoning_summaries);
        assert!(!capabilities.structured_output);
        assert_eq!(capabilities.context_window, None);
    }

    #[test]
    fn runtime_error_display_is_actionable() {
        assert_eq!(
            ModelRuntimeError::StreamInterrupted("idle timeout".to_string()).to_string(),
            "stream interrupted before completion: idle timeout"
        );
    }
}
