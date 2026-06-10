use futures::Stream;
use runtime_protocol::models::ResponseItem;
use runtime_protocol::protocol::TokenUsage;
use serde_json::Value;
use std::pin::Pin;

use crate::provider::ModelRuntimeError;

pub type ModelEventStream =
    Pin<Box<dyn Stream<Item = Result<ModelStreamEvent, ModelRuntimeError>> + Send>>;

#[derive(Debug, Clone, PartialEq)]
pub enum ModelStreamEvent {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ReasoningSummaryDelta {
        text: String,
        summary_index: i64,
    },
    ToolCallStarted {
        call_id: String,
        name: String,
        namespace: Option<String>,
    },
    ToolArgumentsDelta {
        call_id: String,
        delta: String,
    },
    ToolCallCompleted {
        call_id: String,
        name: String,
        namespace: Option<String>,
    },
    StructuredOutput {
        value: Value,
    },
    ResponseItem(ResponseItem),
    Usage(TokenUsage),
    Completed {
        stop_reason: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::ContentItem;

    #[test]
    fn structured_output_event_preserves_json_payload() {
        let event = ModelStreamEvent::StructuredOutput {
            value: serde_json::json!({ "answer": 42 }),
        };

        assert_eq!(
            event,
            ModelStreamEvent::StructuredOutput {
                value: serde_json::json!({ "answer": 42 }),
            }
        );
    }

    #[test]
    fn response_item_event_wraps_message_items() {
        let item = ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: "done".to_string(),
            }],
            phase: None,
        };

        assert_eq!(
            ModelStreamEvent::ResponseItem(item.clone()),
            ModelStreamEvent::ResponseItem(item)
        );
    }
}
