use std::path::PathBuf;

use runtime_protocol::models::PermissionProfile;
use runtime_protocol::protocol::AskForApproval;
use runtime_protocol::protocol::SessionSource;
use tracing::Span;
use tracing::field;
use tracing::info_span;
use utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone)]
pub struct RuntimeTraceMetadata {
    pub thread_id: String,
    pub agent_path: String,
    pub task_name: Option<String>,
    pub nickname: Option<String>,
    pub agent_role: Option<String>,
    pub session_source: SessionSource,
    pub cwd: AbsolutePathBuf,
    pub rollout_path: Option<PathBuf>,
    pub model: String,
    pub provider_name: String,
    pub approval_policy: AskForApproval,
    pub permission_profile: PermissionProfile,
}

#[derive(Debug, Clone)]
pub struct RuntimeTrace {
    thread_id: String,
    thread_span: Span,
    #[cfg(feature = "runtime-diagnostics")]
    rollout: rollout_trace::ThreadTraceContext,
}

impl RuntimeTrace {
    pub fn disabled(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            thread_span: Span::none(),
            #[cfg(feature = "runtime-diagnostics")]
            rollout: rollout_trace::ThreadTraceContext::disabled(),
        }
    }

    pub fn start_root(metadata: RuntimeTraceMetadata) -> Self {
        let thread_span = thread_span(&metadata);
        #[cfg(feature = "runtime-diagnostics")]
        let rollout =
            rollout_trace::ThreadTraceContext::start_root_or_disabled(rollout_metadata(&metadata));
        Self {
            thread_id: metadata.thread_id,
            thread_span,
            #[cfg(feature = "runtime-diagnostics")]
            rollout,
        }
    }

    pub fn start_turn(&self, turn_id: impl Into<String>) -> RuntimeTurnTrace {
        let turn_id = turn_id.into();
        let span = info_span!(
            parent: &self.thread_span,
            "runtime.turn",
            thread.id = %self.thread_id,
            turn.id = %turn_id,
            tool.call_id = field::Empty,
        );
        #[cfg(feature = "runtime-diagnostics")]
        self.rollout
            .record_runtime_activation_started(turn_id.clone());
        RuntimeTurnTrace {
            thread_id: self.thread_id.clone(),
            turn_id,
            span,
            #[cfg(feature = "runtime-diagnostics")]
            rollout: self.rollout.clone(),
        }
    }

    #[cfg(feature = "runtime-diagnostics")]
    pub fn rollout_trace(&self) -> &rollout_trace::ThreadTraceContext {
        &self.rollout
    }

    pub fn thread_span(&self) -> &Span {
        &self.thread_span
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeTurnTrace {
    thread_id: String,
    turn_id: String,
    span: Span,
    #[cfg(feature = "runtime-diagnostics")]
    rollout: rollout_trace::ThreadTraceContext,
}

impl RuntimeTurnTrace {
    pub fn start_tool(&self, tool_call_id: impl Into<String>, tool_name: &str) -> RuntimeToolTrace {
        let tool_call_id = tool_call_id.into();
        let span = info_span!(
            parent: &self.span,
            "runtime.tool",
            thread.id = %self.thread_id,
            turn.id = %self.turn_id,
            tool.call_id = %tool_call_id,
            tool.name = %tool_name,
        );
        RuntimeToolTrace { tool_call_id, span }
    }

    pub fn span(&self) -> &Span {
        &self.span
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    #[cfg(feature = "runtime-diagnostics")]
    pub fn rollout_trace(&self) -> &rollout_trace::ThreadTraceContext {
        &self.rollout
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeToolTrace {
    tool_call_id: String,
    span: Span,
}

impl RuntimeToolTrace {
    pub fn span(&self) -> &Span {
        &self.span
    }

    pub fn tool_call_id(&self) -> &str {
        &self.tool_call_id
    }
}

fn thread_span(metadata: &RuntimeTraceMetadata) -> Span {
    info_span!(
        "runtime.thread",
        thread.id = %metadata.thread_id,
        agent.path = %metadata.agent_path,
        agent.task_name = metadata.task_name.as_deref().unwrap_or_default(),
        agent.nickname = metadata.nickname.as_deref().unwrap_or_default(),
        agent.role = metadata.agent_role.as_deref().unwrap_or_default(),
        session.source = %metadata.session_source,
        cwd = %metadata.cwd.as_path().display(),
        model = %metadata.model,
        model.provider = %metadata.provider_name,
        approval.policy = %metadata.approval_policy,
        permission.profile = ?metadata.permission_profile,
    )
}

#[cfg(feature = "runtime-diagnostics")]
fn rollout_metadata(metadata: &RuntimeTraceMetadata) -> rollout_trace::ThreadStartedTraceMetadata {
    rollout_trace::ThreadStartedTraceMetadata {
        thread_id: metadata.thread_id.clone(),
        agent_path: metadata.agent_path.clone(),
        task_name: metadata.task_name.clone(),
        nickname: metadata.nickname.clone(),
        agent_role: metadata.agent_role.clone(),
        session_source: metadata.session_source.clone(),
        cwd: metadata.cwd.clone().into_path_buf(),
        rollout_path: metadata.rollout_path.clone(),
        model: metadata.model.clone(),
        provider_name: metadata.provider_name.clone(),
        approval_policy: metadata.approval_policy.to_string(),
        sandbox_policy: format!("{:?}", metadata.permission_profile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::PermissionProfile;
    use runtime_protocol::protocol::SessionSource;

    #[test]
    fn runtime_trace_creates_correlated_turn_and_tool_handles() {
        let cwd = AbsolutePathBuf::current_dir().expect("cwd");
        let trace = RuntimeTrace::start_root(RuntimeTraceMetadata {
            thread_id: "thread-1".to_string(),
            agent_path: "main".to_string(),
            task_name: None,
            nickname: None,
            agent_role: None,
            session_source: SessionSource::Exec,
            cwd,
            rollout_path: None,
            model: config::DEFAULT_MODEL.to_string(),
            provider_name: "ollama".to_string(),
            approval_policy: AskForApproval::Never,
            permission_profile: PermissionProfile::read_only(),
        });

        let turn = trace.start_turn("turn-1");
        let tool = turn.start_tool("tool-1", "shell");

        assert_eq!(turn.turn_id(), "turn-1");
        assert_eq!(tool.tool_call_id(), "tool-1");
    }
}
