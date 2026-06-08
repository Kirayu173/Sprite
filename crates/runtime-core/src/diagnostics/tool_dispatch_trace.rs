//! Runtime-core adapter for rollout-trace tool dispatch events.

use std::sync::Arc;

use rollout_trace::ExecutionStatus;
use rollout_trace::ToolDispatchInvocation;
use rollout_trace::ToolDispatchPayload;
use rollout_trace::ToolDispatchRequester;
use rollout_trace::ToolDispatchResult;
use rollout_trace::ToolDispatchTraceContext;
use rollout_trace::TraceWriter;
use runtime_protocol::models::AdditionalPermissionProfile;
use runtime_protocol::models::FunctionCallOutputPayload;
use runtime_protocol::models::ResponseInputItem;
use runtime_protocol::models::SandboxPermissions;
use runtime_protocol::models::SearchToolCallParams;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone)]
pub struct RuntimeToolInvocation {
    pub thread_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub tool_namespace: Option<String>,
    pub requester: RuntimeToolRequester,
    pub payload: RuntimeToolPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeToolRequester {
    Model {
        model_visible_call_id: String,
    },
    CodeCell {
        runtime_cell_id: String,
        runtime_tool_call_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeToolPayload {
    Function {
        arguments: String,
    },
    ToolSearch {
        arguments: SearchToolCallParams,
    },
    Custom {
        input: String,
    },
    LocalShell {
        command: Vec<String>,
        workdir: Option<String>,
        timeout_ms: Option<u64>,
        sandbox_permissions: Option<SandboxPermissions>,
        prefix_rule: Option<Vec<String>>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        justification: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ToolDispatchTrace {
    context: ToolDispatchTraceContext,
}

impl ToolDispatchTrace {
    pub fn disabled() -> Self {
        Self {
            context: ToolDispatchTraceContext::disabled(),
        }
    }

    pub fn start(writer: Arc<TraceWriter>, invocation: RuntimeToolInvocation) -> Self {
        let invocation = ToolDispatchInvocation {
            thread_id: invocation.thread_id,
            runtime_activation_id: invocation.turn_id,
            tool_call_id: invocation.tool_call_id,
            tool_name: invocation.tool_name,
            tool_namespace: invocation.tool_namespace,
            requester: match invocation.requester {
                RuntimeToolRequester::Model {
                    model_visible_call_id,
                } => ToolDispatchRequester::Model {
                    model_visible_call_id,
                },
                RuntimeToolRequester::CodeCell {
                    runtime_cell_id,
                    runtime_tool_call_id,
                } => ToolDispatchRequester::CodeCell {
                    runtime_cell_id,
                    runtime_tool_call_id,
                },
            },
            payload: match invocation.payload {
                RuntimeToolPayload::Function { arguments } => {
                    ToolDispatchPayload::Function { arguments }
                }
                RuntimeToolPayload::ToolSearch { arguments } => {
                    ToolDispatchPayload::ToolSearch { arguments }
                }
                RuntimeToolPayload::Custom { input } => ToolDispatchPayload::Custom { input },
                RuntimeToolPayload::LocalShell {
                    command,
                    workdir,
                    timeout_ms,
                    sandbox_permissions,
                    prefix_rule,
                    additional_permissions,
                    justification,
                } => ToolDispatchPayload::LocalShell {
                    command,
                    workdir,
                    timeout_ms,
                    sandbox_permissions,
                    prefix_rule,
                    additional_permissions,
                    justification,
                },
            },
        };
        Self {
            context: ToolDispatchTraceContext::start(writer, invocation),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.context.is_enabled()
    }

    pub fn record_function_output(&self, call_id: String, output: FunctionCallOutputPayload) {
        self.record_direct_response(ResponseInputItem::FunctionCallOutput { call_id, output });
    }

    pub fn record_direct_response(&self, response_item: ResponseInputItem) {
        self.context.record_completed(
            ExecutionStatus::Completed,
            ToolDispatchResult::DirectResponse { response_item },
        );
    }

    pub fn record_code_mode_response(&self, value: JsonValue) {
        self.context.record_completed(
            ExecutionStatus::Completed,
            ToolDispatchResult::CodeModeResponse { value },
        );
    }

    pub fn record_completed(&self, result: ToolDispatchResult) {
        self.context
            .record_completed(ExecutionStatus::Completed, result);
    }

    pub fn record_failed(&self, error: impl std::fmt::Display) {
        self.context.record_failed(error);
    }
}
