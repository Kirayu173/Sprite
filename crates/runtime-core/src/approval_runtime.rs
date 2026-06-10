use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use config::{HookHandlerConfig, HooksToml};
use exec_server::{ExecEnvPolicy, ExecServerRuntimePaths, TerminalSize};
use runtime_protocol::approvals::{
    ApplyPatchApprovalRequestEvent, ExecApprovalRequestEvent, NetworkPolicyAmendment,
    NetworkPolicyRuleAction,
};
use runtime_protocol::config_types::{ApprovalsReviewer, WindowsSandboxLevel};
use runtime_protocol::exec_output::ExecToolCallOutput;
use runtime_protocol::models::{PermissionProfile, SandboxPermissions};
use runtime_protocol::parse_command::ParsedCommand;
use runtime_protocol::protocol::{
    AskForApproval, EventMsg, ExecCommandBeginEvent, ExecCommandEndEvent, ExecCommandSource,
    ExecCommandStatus, FileChange, HookCompletedEvent, HookEventName, HookExecutionMode,
    HookHandlerType, HookOutputEntry, HookOutputEntryKind, HookRunStatus, HookRunSummary,
    HookScope, HookSource, HookStartedEvent, PatchApplyBeginEvent, PatchApplyEndEvent,
    PatchApplyStatus, ReviewDecision,
};
use runtime_protocol::request_permissions::{
    RequestPermissionProfile, RequestPermissionsEvent, RequestPermissionsResponse,
};
use sandboxing::{SandboxCommand, SandboxManager, SandboxType, SandboxablePreference};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use utils_absolute_path::AbsolutePathBuf;

use crate::exec::{ExecExpiration, LocalExecParams, execute_local_command};
use crate::exec_policy::{ExecApprovalRequest, ExecPolicyManager};
use crate::network_policy_decision::{
    ExecPolicyNetworkRuleAmendment, execpolicy_network_rule_amendment,
};
use crate::safety::{PatchAction, SafetyCheck, assess_patch_safety};
use crate::tools::network_approval::{
    NetworkApprovalOutcome, NetworkApprovalRequest, NetworkApprovalService,
    SpriteAutomatedReviewer, network_approval_context, review_permissions_request,
};
use crate::tools::sandboxing::{
    ApprovalStore, ExecApprovalRequirement, PermissionRequestPayload, SandboxOverride,
    build_sandbox_exec_request, managed_network_for_sandbox_permissions,
    permission_request_payload, sandbox_override_for_first_attempt,
    sandbox_permissions_preserving_denied_reads, select_initial_sandbox,
};

#[derive(Debug, Clone)]
pub struct ApprovalRuntimeContext {
    pub session_id: String,
    pub turn_id: String,
    pub model: String,
    pub cwd: AbsolutePathBuf,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub permission_profile: PermissionProfile,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
    pub runtime_paths: ExecServerRuntimePaths,
    pub sprite_home: PathBuf,
    pub sprite_linux_sandbox_exe: Option<PathBuf>,
    pub hooks: HooksToml,
}

#[derive(Debug, Clone)]
pub struct CommandApprovalRequest {
    pub call_id: String,
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub env: HashMap<String, String>,
    pub env_policy: Option<ExecEnvPolicy>,
    pub terminal_size: TerminalSize,
    pub expiration: ExecExpiration,
    pub arg0: Option<String>,
    pub description: Option<String>,
    pub sandbox_permissions: SandboxPermissions,
    pub prefix_rule: Option<Vec<String>>,
    pub source: ExecCommandSource,
}

#[derive(Debug, Clone)]
pub struct PatchApprovalRequest {
    pub call_id: String,
    pub action: PatchAction,
    pub changes: HashMap<PathBuf, FileChange>,
}

#[derive(Debug, Clone)]
pub struct NetworkAccessRequest {
    pub call_id: String,
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub request: NetworkApprovalRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    Allow,
    Deny { message: String },
}

#[derive(Debug, Clone)]
pub enum CommandExecutionOutcome {
    Executed {
        output: ExecToolCallOutput,
        events: Vec<EventMsg>,
        sandbox: SandboxType,
    },
    Declined {
        reason: String,
        events: Vec<EventMsg>,
    },
}

#[derive(Debug, Clone)]
pub enum PatchExecutionOutcome {
    Applied {
        output: ExecToolCallOutput,
        events: Vec<EventMsg>,
        auto_approved: bool,
    },
    Declined {
        reason: String,
        events: Vec<EventMsg>,
    },
}

#[derive(Debug, Clone)]
pub enum NetworkExecutionOutcome {
    AllowOnce {
        events: Vec<EventMsg>,
    },
    AllowForSession {
        events: Vec<EventMsg>,
    },
    Denied {
        reason: String,
        events: Vec<EventMsg>,
    },
}

pub trait ApprovalDecisionProvider {
    fn review_exec(&mut self, event: &ExecApprovalRequestEvent) -> ReviewDecision;
    fn review_patch(&mut self, event: &ApplyPatchApprovalRequestEvent) -> ReviewDecision;
    fn review_permissions(&mut self, event: &RequestPermissionsEvent)
    -> RequestPermissionsResponse;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct CommandApprovalCacheKey {
    command: Vec<String>,
    cwd: AbsolutePathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct PatchApprovalCacheKey {
    cwd: AbsolutePathBuf,
    path: PathBuf,
}

pub struct ApprovalRuntime {
    sandbox_manager: SandboxManager,
    command_approvals: ApprovalStore,
    patch_approvals: ApprovalStore,
    network_approvals: NetworkApprovalService<SpriteAutomatedReviewer>,
}

impl Default for ApprovalRuntime {
    fn default() -> Self {
        Self {
            sandbox_manager: SandboxManager::default(),
            command_approvals: ApprovalStore::default(),
            patch_approvals: ApprovalStore::default(),
            network_approvals: NetworkApprovalService::new(SpriteAutomatedReviewer),
        }
    }
}

impl ApprovalRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn execute_command(
        &mut self,
        ctx: &ApprovalRuntimeContext,
        exec_policy: &ExecPolicyManager,
        backend: &mut dyn ApprovalDecisionProvider,
        req: CommandApprovalRequest,
    ) -> Result<CommandExecutionOutcome, String> {
        let mut events = Vec::new();
        if req.command.is_empty() {
            return Err("command must not be empty".to_string());
        }

        let cache_key = CommandApprovalCacheKey {
            command: req.command.clone(),
            cwd: req.cwd.clone(),
        };
        let fs_policy = ctx.permission_profile.file_system_sandbox_policy();
        let mut approval_requirement = exec_policy
            .create_exec_approval_requirement_for_command(ExecApprovalRequest {
                command: &req.command,
                approval_policy: ctx.approval_policy,
                permission_profile: ctx.permission_profile.clone(),
                windows_sandbox_level: ctx.windows_sandbox_level,
                sandbox_permissions: req.sandbox_permissions,
                prefix_rule: req.prefix_rule.clone(),
            })
            .await;

        if matches!(
            approval_requirement,
            ExecApprovalRequirement::NeedsApproval { .. }
        ) {
            let hook_outcome = self
                .run_permission_request_hooks(
                    ctx,
                    permission_request_payload(
                        req.command.join(" "),
                        req.description.clone(),
                        None,
                    ),
                )
                .await?;
            events.extend(hook_outcome.events);

            match hook_outcome.decision {
                Some(HookDecision::Allow) => {
                    approval_requirement = ExecApprovalRequirement::Skip {
                        bypass_sandbox: false,
                        proposed_execpolicy_amendment: approval_requirement
                            .proposed_execpolicy_amendment()
                            .cloned(),
                    };
                }
                Some(HookDecision::Deny { message }) => {
                    return Ok(CommandExecutionOutcome::Declined {
                        reason: message,
                        events,
                    });
                }
                None => {}
            }
        }

        match &approval_requirement {
            ExecApprovalRequirement::Forbidden { reason } => {
                return Ok(CommandExecutionOutcome::Declined {
                    reason: reason.clone(),
                    events,
                });
            }
            ExecApprovalRequirement::NeedsApproval { .. }
                if matches!(
                    self.command_approvals.get(&cache_key),
                    Some(ReviewDecision::ApprovedForSession)
                ) =>
            {
                approval_requirement = ExecApprovalRequirement::Skip {
                    bypass_sandbox: false,
                    proposed_execpolicy_amendment: approval_requirement
                        .proposed_execpolicy_amendment()
                        .cloned(),
                };
            }
            ExecApprovalRequirement::NeedsApproval {
                reason,
                proposed_execpolicy_amendment,
            } => {
                let approval_event = ExecApprovalRequestEvent {
                    call_id: req.call_id.clone(),
                    approval_id: None,
                    turn_id: ctx.turn_id.clone(),
                    started_at_ms: now_ms(),
                    command: req.command.clone(),
                    cwd: req.cwd.clone(),
                    reason: reason.clone(),
                    network_approval_context: None,
                    proposed_execpolicy_amendment: proposed_execpolicy_amendment.clone(),
                    proposed_network_policy_amendments: None,
                    additional_permissions: None,
                    available_decisions: None,
                    parsed_cmd: vec![ParsedCommand::Unknown {
                        cmd: req.command.join(" "),
                    }],
                };
                events.push(EventMsg::ExecApprovalRequest(approval_event.clone()));
                match backend.review_exec(&approval_event) {
                    ReviewDecision::Approved => {}
                    ReviewDecision::ApprovedForSession => {
                        self.command_approvals
                            .put(cache_key.clone(), ReviewDecision::ApprovedForSession);
                    }
                    ReviewDecision::ApprovedExecpolicyAmendment {
                        proposed_execpolicy_amendment,
                    } => {
                        exec_policy
                            .append_amendment_and_update(
                                ctx.sprite_home.as_path(),
                                &proposed_execpolicy_amendment,
                            )
                            .await
                            .map_err(|err| err.to_string())?;
                    }
                    ReviewDecision::Denied
                    | ReviewDecision::Abort
                    | ReviewDecision::TimedOut
                    | ReviewDecision::NetworkPolicyAmendment { .. } => {
                        return Ok(CommandExecutionOutcome::Declined {
                            reason: "command execution was not approved".to_string(),
                            events,
                        });
                    }
                }
            }
            ExecApprovalRequirement::Skip { .. } => {}
        }

        let sandbox_permissions =
            sandbox_permissions_preserving_denied_reads(req.sandbox_permissions, &fs_policy);
        let managed_network = managed_network_for_sandbox_permissions(None, sandbox_permissions);
        let selected_sandbox = match sandbox_override_for_first_attempt(
            sandbox_permissions,
            &approval_requirement,
            &fs_policy,
        ) {
            SandboxOverride::BypassSandboxFirstAttempt => SandboxType::None,
            SandboxOverride::NoOverride => select_initial_sandbox(
                &self.sandbox_manager,
                &ctx.permission_profile,
                SandboxablePreference::Auto,
                ctx.windows_sandbox_level,
                managed_network,
            ),
        };

        let sandbox_request = build_sandbox_exec_request(
            &self.sandbox_manager,
            build_sandbox_command(&req.command, &req.cwd, req.env.clone())?,
            &ctx.permission_profile,
            selected_sandbox,
            false,
            managed_network,
            req.cwd.as_path(),
            ctx.sprite_linux_sandbox_exe.as_deref(),
            ctx.windows_sandbox_level,
            ctx.windows_sandbox_private_desktop,
        )
        .map_err(|err| err.to_string())?;

        let begin = ExecCommandBeginEvent {
            call_id: req.call_id.clone(),
            process_id: None,
            turn_id: ctx.turn_id.clone(),
            started_at_ms: now_ms(),
            command: sandbox_request.command.clone(),
            cwd: sandbox_request.cwd.clone(),
            parsed_cmd: vec![ParsedCommand::Unknown {
                cmd: req.command.join(" "),
            }],
            source: req.source,
            interaction_input: None,
        };
        events.push(EventMsg::ExecCommandBegin(begin));

        let started = Instant::now();
        let output = execute_local_command(LocalExecParams {
            command: sandbox_request.command.clone(),
            cwd: sandbox_request.cwd.clone(),
            env: sandbox_request.env,
            env_policy: req.env_policy,
            terminal_size: req.terminal_size,
            expiration: req.expiration,
            arg0: sandbox_request.arg0.or(req.arg0),
            runtime_paths: ctx.runtime_paths.clone(),
        })
        .await
        .map_err(|err| err.to_string())?;

        let status = if output.exit_code == 0 {
            ExecCommandStatus::Completed
        } else {
            ExecCommandStatus::Failed
        };
        events.push(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
            call_id: req.call_id,
            process_id: None,
            turn_id: ctx.turn_id.clone(),
            completed_at_ms: now_ms(),
            command: req.command,
            cwd: req.cwd,
            parsed_cmd: vec![ParsedCommand::Unknown {
                cmd: sandbox_request.command.join(" "),
            }],
            source: req.source,
            interaction_input: None,
            stdout: output.stdout.text.clone(),
            stderr: output.stderr.text.clone(),
            aggregated_output: output.aggregated_output.text.clone(),
            exit_code: output.exit_code,
            duration: started.elapsed(),
            formatted_output: output.aggregated_output.text.clone(),
            status,
        }));

        Ok(CommandExecutionOutcome::Executed {
            output,
            events,
            sandbox: selected_sandbox,
        })
    }

    pub async fn apply_patch<F, Fut>(
        &mut self,
        ctx: &ApprovalRuntimeContext,
        backend: &mut dyn ApprovalDecisionProvider,
        req: PatchApprovalRequest,
        apply: F,
    ) -> Result<PatchExecutionOutcome, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<ExecToolCallOutput, String>>,
    {
        let mut events = Vec::new();
        let fs_policy = ctx.permission_profile.file_system_sandbox_policy();
        let safety = assess_patch_safety(
            &req.action,
            ctx.approval_policy,
            &ctx.permission_profile,
            &fs_policy,
            &ctx.cwd,
            ctx.windows_sandbox_level,
        );

        let approval_keys: Vec<PatchApprovalCacheKey> = req
            .changes
            .keys()
            .cloned()
            .map(|path| PatchApprovalCacheKey {
                cwd: ctx.cwd.clone(),
                path,
            })
            .collect();

        let auto_approved = match safety {
            SafetyCheck::AutoApprove { .. } => true,
            SafetyCheck::Reject { reason } => {
                events.push(EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                    call_id: req.call_id,
                    turn_id: ctx.turn_id.clone(),
                    stdout: String::new(),
                    stderr: reason.clone(),
                    success: false,
                    changes: req.changes,
                    status: PatchApplyStatus::Declined,
                }));
                return Ok(PatchExecutionOutcome::Declined { reason, events });
            }
            SafetyCheck::AskUser => false,
        };

        let approved = if auto_approved {
            true
        } else if approval_keys.iter().all(|key| {
            matches!(
                self.patch_approvals.get(key),
                Some(ReviewDecision::ApprovedForSession)
            )
        }) {
            true
        } else {
            let hook_outcome = self
                .run_permission_request_hooks(
                    ctx,
                    HookPayload {
                        tool_name: "apply_patch".to_string(),
                        matcher_aliases: vec!["apply_patch".to_string()],
                        tool_input: serde_json::to_value(&req.changes).unwrap_or(Value::Null),
                        run_id_suffix: format!("{}:patch", req.call_id),
                    },
                )
                .await?;
            events.extend(hook_outcome.events);

            if let Some(decision) = hook_outcome.decision {
                match decision {
                    HookDecision::Allow => true,
                    HookDecision::Deny { message } => {
                        events.push(EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                            call_id: req.call_id,
                            turn_id: ctx.turn_id.clone(),
                            stdout: String::new(),
                            stderr: message.clone(),
                            success: false,
                            changes: req.changes,
                            status: PatchApplyStatus::Declined,
                        }));
                        return Ok(PatchExecutionOutcome::Declined {
                            reason: message,
                            events,
                        });
                    }
                }
            } else {
                let approval_event = ApplyPatchApprovalRequestEvent {
                    call_id: req.call_id.clone(),
                    turn_id: ctx.turn_id.clone(),
                    started_at_ms: now_ms(),
                    changes: req.changes.clone(),
                    reason: None,
                    grant_root: None,
                };
                events.push(EventMsg::ApplyPatchApprovalRequest(approval_event.clone()));
                match backend.review_patch(&approval_event) {
                    ReviewDecision::Approved => true,
                    ReviewDecision::ApprovedForSession => {
                        for key in approval_keys {
                            self.patch_approvals
                                .put(key, ReviewDecision::ApprovedForSession);
                        }
                        true
                    }
                    ReviewDecision::Denied
                    | ReviewDecision::Abort
                    | ReviewDecision::TimedOut
                    | ReviewDecision::ApprovedExecpolicyAmendment { .. }
                    | ReviewDecision::NetworkPolicyAmendment { .. } => false,
                }
            }
        };

        if !approved {
            events.push(EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                call_id: req.call_id,
                turn_id: ctx.turn_id.clone(),
                stdout: String::new(),
                stderr: "patch application was not approved".to_string(),
                success: false,
                changes: req.changes,
                status: PatchApplyStatus::Declined,
            }));
            return Ok(PatchExecutionOutcome::Declined {
                reason: "patch application was not approved".to_string(),
                events,
            });
        }

        events.push(EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
            call_id: req.call_id.clone(),
            turn_id: ctx.turn_id.clone(),
            auto_approved,
            changes: req.changes.clone(),
        }));

        let output = apply().await?;
        let status = if output.exit_code == 0 {
            PatchApplyStatus::Completed
        } else {
            PatchApplyStatus::Failed
        };
        let success = matches!(status, PatchApplyStatus::Completed);
        events.push(EventMsg::PatchApplyEnd(PatchApplyEndEvent {
            call_id: req.call_id,
            turn_id: ctx.turn_id.clone(),
            stdout: output.stdout.text.clone(),
            stderr: output.stderr.text.clone(),
            success,
            changes: req.changes,
            status,
        }));

        Ok(PatchExecutionOutcome::Applied {
            output,
            events,
            auto_approved,
        })
    }

    pub async fn handle_network_access(
        &mut self,
        ctx: &ApprovalRuntimeContext,
        exec_policy: &ExecPolicyManager,
        backend: &mut dyn ApprovalDecisionProvider,
        req: NetworkAccessRequest,
    ) -> Result<NetworkExecutionOutcome, String> {
        let mut events = Vec::new();

        if matches!(ctx.approvals_reviewer, ApprovalsReviewer::AutoReview) {
            match self
                .network_approvals
                .evaluate(&req.request, ctx.approvals_reviewer)
            {
                NetworkApprovalOutcome::AllowOnce => {
                    return Ok(NetworkExecutionOutcome::AllowOnce { events });
                }
                NetworkApprovalOutcome::AllowForSession => {
                    return Ok(NetworkExecutionOutcome::AllowForSession { events });
                }
                NetworkApprovalOutcome::Deny => {
                    return Ok(NetworkExecutionOutcome::Denied {
                        reason: "network access was denied by automatic review".to_string(),
                        events,
                    });
                }
                NetworkApprovalOutcome::DenyByPolicy(reason) => {
                    return Ok(NetworkExecutionOutcome::Denied { reason, events });
                }
            }
        }

        if self.network_approvals.cache().is_denied(&req.request) {
            return Ok(NetworkExecutionOutcome::Denied {
                reason: "network access was previously denied for this session".to_string(),
                events,
            });
        }
        if self.network_approvals.cache().is_approved(&req.request) {
            return Ok(NetworkExecutionOutcome::AllowForSession { events });
        }

        let hook_outcome = self
            .run_permission_request_hooks(
                ctx,
                HookPayload {
                    tool_name: "shell".to_string(),
                    matcher_aliases: vec!["shell".to_string(), "bash".to_string()],
                    tool_input: serde_json::json!({
                        "command": req.command,
                        "network": {
                            "target": req.request.target,
                            "host": req.request.host,
                            "protocol": format!("{:?}", req.request.protocol),
                            "port": req.request.port,
                        }
                    }),
                    run_id_suffix: format!("{}:network", req.call_id),
                },
            )
            .await?;
        events.extend(hook_outcome.events);
        if let Some(decision) = hook_outcome.decision {
            return Ok(match decision {
                HookDecision::Allow => NetworkExecutionOutcome::AllowOnce { events },
                HookDecision::Deny { message } => NetworkExecutionOutcome::Denied {
                    reason: message,
                    events,
                },
            });
        }

        let network_context = network_approval_context(&req.request);
        let proposed_network_policy_amendments = Some(vec![
            NetworkPolicyAmendment {
                host: req.request.host.clone(),
                action: NetworkPolicyRuleAction::Allow,
            },
            NetworkPolicyAmendment {
                host: req.request.host.clone(),
                action: NetworkPolicyRuleAction::Deny,
            },
        ]);
        let event = ExecApprovalRequestEvent {
            call_id: req.call_id.clone(),
            approval_id: None,
            turn_id: ctx.turn_id.clone(),
            started_at_ms: now_ms(),
            command: req.command.clone(),
            cwd: req.cwd.clone(),
            reason: req.request.reason.clone(),
            network_approval_context: Some(network_context.clone()),
            proposed_execpolicy_amendment: None,
            proposed_network_policy_amendments: proposed_network_policy_amendments.clone(),
            additional_permissions: None,
            available_decisions: Some(ExecApprovalRequestEvent::default_available_decisions(
                Some(&network_context),
                None,
                proposed_network_policy_amendments.as_deref(),
                None,
            )),
            parsed_cmd: vec![ParsedCommand::Unknown {
                cmd: req.command.join(" "),
            }],
        };
        events.push(EventMsg::ExecApprovalRequest(event.clone()));

        match backend.review_exec(&event) {
            ReviewDecision::Approved => Ok(NetworkExecutionOutcome::AllowOnce { events }),
            ReviewDecision::ApprovedForSession => {
                self.network_approvals
                    .cache_mut()
                    .allow_for_session(&req.request);
                Ok(NetworkExecutionOutcome::AllowForSession { events })
            }
            ReviewDecision::NetworkPolicyAmendment {
                network_policy_amendment,
            } => {
                let ExecPolicyNetworkRuleAmendment {
                    protocol,
                    decision,
                    justification,
                } = execpolicy_network_rule_amendment(
                    &network_policy_amendment,
                    &network_context,
                    &req.request.host,
                );
                exec_policy
                    .append_network_rule_and_update(
                        ctx.sprite_home.as_path(),
                        &req.request.host,
                        protocol,
                        decision,
                        Some(justification),
                    )
                    .await
                    .map_err(|err| err.to_string())?;
                match network_policy_amendment.action {
                    NetworkPolicyRuleAction::Allow => {
                        Ok(NetworkExecutionOutcome::AllowOnce { events })
                    }
                    NetworkPolicyRuleAction::Deny => Ok(NetworkExecutionOutcome::Denied {
                        reason: "network access was denied and persisted in exec policy"
                            .to_string(),
                        events,
                    }),
                }
            }
            ReviewDecision::Denied
            | ReviewDecision::Abort
            | ReviewDecision::TimedOut
            | ReviewDecision::ApprovedExecpolicyAmendment { .. } => {
                Ok(NetworkExecutionOutcome::Denied {
                    reason: "network access was not approved".to_string(),
                    events,
                })
            }
        }
    }

    pub fn request_additional_permissions(
        &mut self,
        ctx: &ApprovalRuntimeContext,
        backend: &mut dyn ApprovalDecisionProvider,
        call_id: String,
        reason: Option<String>,
        permissions: RequestPermissionProfile,
    ) -> Result<(RequestPermissionsResponse, Vec<EventMsg>), String> {
        let mut events = Vec::new();
        if permissions.is_empty() {
            return Err("requested permissions must not be empty".to_string());
        }

        let event = RequestPermissionsEvent {
            call_id,
            turn_id: ctx.turn_id.clone(),
            environment_id: None,
            started_at_ms: now_ms(),
            reason: reason.clone(),
            permissions: permissions.clone(),
            cwd: Some(ctx.cwd.clone()),
        };
        events.push(EventMsg::RequestPermissions(event.clone()));

        let response = if matches!(ctx.approvals_reviewer, ApprovalsReviewer::AutoReview) {
            review_permissions_request(&SpriteAutomatedReviewer, reason, permissions)
        } else {
            backend.review_permissions(&event)
        };

        Ok((response, events))
    }

    async fn run_permission_request_hooks<P>(
        &self,
        ctx: &ApprovalRuntimeContext,
        payload: P,
    ) -> Result<HookExecutionOutcome, String>
    where
        P: Into<HookPayload>,
    {
        let payload = payload.into();
        let handlers = &ctx.hooks.events.permission_request;
        if handlers.is_empty() {
            return Ok(HookExecutionOutcome::default());
        }

        let mut events = Vec::new();
        let mut decisions = Vec::new();
        for (group_index, group) in handlers.iter().enumerate() {
            if !matcher_matches(
                group.matcher.as_deref(),
                &payload.tool_name,
                &payload.matcher_aliases,
            ) {
                continue;
            }

            for (hook_index, hook) in group.hooks.iter().enumerate() {
                let started_at_ms = now_ms();
                let summary = HookRunSummary {
                    id: format!(
                        "permission-request:{group_index}:{hook_index}:{}",
                        payload.run_id_suffix
                    ),
                    event_name: HookEventName::PermissionRequest,
                    handler_type: handler_type(hook),
                    execution_mode: HookExecutionMode::Sync,
                    scope: HookScope::Turn,
                    source_path: hook_source_path(hook, ctx.cwd.as_path()),
                    source: HookSource::Unknown,
                    display_order: (group_index * 1000 + hook_index) as i64,
                    status: HookRunStatus::Running,
                    status_message: hook_status_message(hook),
                    started_at: started_at_ms,
                    completed_at: None,
                    duration_ms: None,
                    entries: Vec::new(),
                };

                let mut completed = summary.clone();
                let started = Instant::now();
                let mut entries = Vec::new();
                let decision = match hook {
                    HookHandlerConfig::Command {
                        command,
                        command_windows,
                        timeout_sec,
                        ..
                    } => {
                        let run = run_hook_command(
                            command,
                            command_windows.as_deref(),
                            ctx.cwd.as_path(),
                            *timeout_sec,
                            serde_json::json!({
                                "session_id": ctx.session_id,
                                "turn_id": ctx.turn_id,
                                "cwd": ctx.cwd.as_path().display().to_string(),
                                "hook_event_name": "PermissionRequest",
                                "model": ctx.model,
                                "permission_mode": hook_permission_mode(ctx.approval_policy),
                                "tool_name": payload.tool_name,
                                "tool_input": payload.tool_input,
                            }),
                        )
                        .await?;
                        parse_permission_request_hook_result(run, &mut entries)
                    }
                    HookHandlerConfig::Prompt {} | HookHandlerConfig::Agent {} => {
                        entries.push(HookOutputEntry {
                            kind: HookOutputEntryKind::Error,
                            text: "PermissionRequest only supports command hooks".to_string(),
                        });
                        completed.status = HookRunStatus::Failed;
                        None
                    }
                };

                completed.status = if entries.iter().any(|entry| {
                    matches!(
                        entry.kind,
                        HookOutputEntryKind::Error | HookOutputEntryKind::Stop
                    )
                }) {
                    HookRunStatus::Failed
                } else if matches!(decision, Some(HookDecision::Deny { .. })) {
                    HookRunStatus::Blocked
                } else {
                    HookRunStatus::Completed
                };
                completed.entries = entries;
                completed.completed_at = Some(now_ms());
                completed.duration_ms = Some(started.elapsed().as_millis() as i64);

                let turn_id = Some(ctx.turn_id.clone());
                events.push(EventMsg::HookStarted(HookStartedEvent {
                    turn_id: turn_id.clone(),
                    run: summary,
                }));
                events.push(EventMsg::HookCompleted(HookCompletedEvent {
                    turn_id,
                    run: completed,
                }));

                if let Some(decision) = decision {
                    decisions.push(decision);
                }
            }
        }

        for decision in &decisions {
            if matches!(decision, HookDecision::Deny { .. }) {
                return Ok(HookExecutionOutcome {
                    decision: Some(decision.clone()),
                    events,
                });
            }
        }
        Ok(HookExecutionOutcome {
            decision: decisions
                .into_iter()
                .find(|decision| matches!(decision, HookDecision::Allow)),
            events,
        })
    }
}

#[derive(Debug, Default)]
struct HookExecutionOutcome {
    decision: Option<HookDecision>,
    events: Vec<EventMsg>,
}

#[derive(Debug, Clone)]
struct HookPayload {
    tool_name: String,
    matcher_aliases: Vec<String>,
    tool_input: Value,
    run_id_suffix: String,
}

impl From<PermissionRequestPayload> for HookPayload {
    fn from(value: PermissionRequestPayload) -> Self {
        let tool_input = serde_json::json!({
            "command": value.command,
            "description": value.description,
            "network_approval_context": value.network_approval_context,
        });
        Self {
            tool_name: "shell".to_string(),
            matcher_aliases: vec!["shell".to_string(), "bash".to_string()],
            tool_input,
            run_id_suffix: "exec".to_string(),
        }
    }
}

#[derive(Debug)]
struct HookRunResult {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRequestHookOutputWire {
    #[serde(default = "default_true")]
    r#continue: bool,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    suppress_output: bool,
    #[serde(default)]
    system_message: Option<String>,
    #[serde(default)]
    hook_specific_output: Option<PermissionRequestHookSpecificOutputWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRequestHookSpecificOutputWire {
    hook_event_name: String,
    #[serde(default)]
    decision: Option<PermissionRequestDecisionWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRequestDecisionWire {
    behavior: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    updated_input: Option<Value>,
    #[serde(default)]
    updated_permissions: Option<Value>,
    #[serde(default)]
    interrupt: bool,
}

async fn run_hook_command(
    command: &str,
    command_windows: Option<&str>,
    cwd: &Path,
    timeout_sec: Option<u64>,
    input: Value,
) -> Result<HookRunResult, String> {
    let mut process = if cfg!(windows) {
        let shell_command = command_windows.unwrap_or(command);
        let mut process = Command::new("cmd");
        process.arg("/C").arg(shell_command);
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-lc").arg(command);
        process
    };

    process
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = process.spawn().map_err(|err| err.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let input_json = serde_json::to_vec(&input).map_err(|err| err.to_string())?;
        stdin
            .write_all(&input_json)
            .await
            .map_err(|err| err.to_string())?;
    }

    let output = if let Some(timeout_sec) = timeout_sec {
        tokio::time::timeout(Duration::from_secs(timeout_sec), child.wait_with_output())
            .await
            .map_err(|_| "hook timed out".to_string())?
            .map_err(|err| err.to_string())?
    } else {
        child
            .wait_with_output()
            .await
            .map_err(|err| err.to_string())?
    };

    Ok(HookRunResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

fn parse_permission_request_hook_result(
    run: HookRunResult,
    entries: &mut Vec<HookOutputEntry>,
) -> Option<HookDecision> {
    match run.exit_code {
        Some(0) => {
            let trimmed = run.stdout.trim();
            if trimmed.is_empty() {
                return None;
            }

            match serde_json::from_str::<PermissionRequestHookOutputWire>(trimmed) {
                Ok(parsed) => {
                    if let Some(message) = parsed.system_message {
                        entries.push(HookOutputEntry {
                            kind: HookOutputEntryKind::Warning,
                            text: message,
                        });
                    }
                    if !parsed.r#continue || parsed.stop_reason.is_some() || parsed.suppress_output
                    {
                        entries.push(HookOutputEntry {
                            kind: HookOutputEntryKind::Error,
                            text: "PermissionRequest hook returned unsupported universal output"
                                .to_string(),
                        });
                        return None;
                    }
                    let Some(hook_output) = parsed.hook_specific_output else {
                        return None;
                    };
                    if hook_output.hook_event_name != "PermissionRequest" {
                        entries.push(HookOutputEntry {
                            kind: HookOutputEntryKind::Error,
                            text: "PermissionRequest hook returned mismatched hookEventName"
                                .to_string(),
                        });
                        return None;
                    }
                    let Some(decision) = hook_output.decision else {
                        return None;
                    };
                    if decision.updated_input.is_some()
                        || decision.updated_permissions.is_some()
                        || decision.interrupt
                    {
                        entries.push(HookOutputEntry {
                            kind: HookOutputEntryKind::Error,
                            text: "PermissionRequest hook returned unsupported decision fields"
                                .to_string(),
                        });
                        return None;
                    }
                    match decision.behavior.as_str() {
                        "allow" => Some(HookDecision::Allow),
                        "deny" => {
                            let message = decision.message.unwrap_or_else(|| {
                                "PermissionRequest hook denied approval".to_string()
                            });
                            entries.push(HookOutputEntry {
                                kind: HookOutputEntryKind::Notice,
                                text: message.clone(),
                            });
                            Some(HookDecision::Deny { message })
                        }
                        _ => {
                            entries.push(HookOutputEntry {
                                kind: HookOutputEntryKind::Error,
                                text: "PermissionRequest hook returned unknown behavior"
                                    .to_string(),
                            });
                            None
                        }
                    }
                }
                Err(_) => {
                    if looks_like_json(trimmed) {
                        entries.push(HookOutputEntry {
                            kind: HookOutputEntryKind::Error,
                            text: "hook returned invalid permission-request JSON output"
                                .to_string(),
                        });
                    }
                    None
                }
            }
        }
        Some(2) => {
            let message = trimmed_non_empty(&run.stderr)
                .unwrap_or_else(|| "PermissionRequest hook denied approval".to_string());
            entries.push(HookOutputEntry {
                kind: HookOutputEntryKind::Notice,
                text: message.clone(),
            });
            Some(HookDecision::Deny { message })
        }
        Some(code) => {
            let detail = trimmed_non_empty(&run.stderr)
                .map(|stderr| format!(": {stderr}"))
                .unwrap_or_default();
            entries.push(HookOutputEntry {
                kind: HookOutputEntryKind::Error,
                text: format!("hook exited with code {code}{detail}"),
            });
            None
        }
        None => {
            entries.push(HookOutputEntry {
                kind: HookOutputEntryKind::Error,
                text: "hook exited without a status code".to_string(),
            });
            None
        }
    }
}

fn build_sandbox_command(
    command: &[String],
    cwd: &AbsolutePathBuf,
    env: HashMap<String, String>,
) -> Result<SandboxCommand, String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| "command must not be empty".to_string())?;
    Ok(SandboxCommand {
        program: OsString::from(program),
        args: args.to_vec(),
        cwd: cwd.clone(),
        env,
        additional_permissions: None,
    })
}

fn matcher_matches(matcher: Option<&str>, tool_name: &str, aliases: &[String]) -> bool {
    let Some(matcher) = matcher else {
        return true;
    };
    if matcher == "*" {
        return true;
    }
    if matcher.eq_ignore_ascii_case(tool_name) {
        return true;
    }
    aliases
        .iter()
        .any(|alias| matcher.eq_ignore_ascii_case(alias.as_str()))
}

fn hook_source_path(hook: &HookHandlerConfig, cwd: &Path) -> AbsolutePathBuf {
    let path = match hook {
        HookHandlerConfig::Command {
            command,
            command_windows,
            ..
        } => {
            let raw = if cfg!(windows) {
                command_windows.as_deref().unwrap_or(command.as_str())
            } else {
                command.as_str()
            };
            raw.split_whitespace()
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| cwd.join("hooks"))
        }
        HookHandlerConfig::Prompt {} | HookHandlerConfig::Agent {} => cwd.join("hooks"),
    };
    AbsolutePathBuf::from_absolute_path(
        dunce::canonicalize(path).unwrap_or_else(|_| cwd.to_path_buf()),
    )
    .unwrap_or_else(|_| {
        AbsolutePathBuf::from_absolute_path(cwd.to_path_buf()).expect("cwd is absolute")
    })
}

fn handler_type(hook: &HookHandlerConfig) -> HookHandlerType {
    match hook {
        HookHandlerConfig::Command { .. } => HookHandlerType::Command,
        HookHandlerConfig::Prompt {} => HookHandlerType::Prompt,
        HookHandlerConfig::Agent {} => HookHandlerType::Agent,
    }
}

fn hook_status_message(hook: &HookHandlerConfig) -> Option<String> {
    match hook {
        HookHandlerConfig::Command { status_message, .. } => status_message.clone(),
        HookHandlerConfig::Prompt {} | HookHandlerConfig::Agent {} => None,
    }
}

fn hook_permission_mode(policy: AskForApproval) -> &'static str {
    match policy {
        AskForApproval::Never => "bypassPermissions",
        AskForApproval::OnFailure
        | AskForApproval::OnRequest
        | AskForApproval::UnlessTrusted
        | AskForApproval::Granular(_) => "default",
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn looks_like_json(value: &str) -> bool {
    let trimmed = value.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn trimmed_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExecExpiration;
    use crate::exec_policy::ExecPolicyManager;
    use config::RuntimeConfigBuilder;
    use runtime_protocol::exec_output::StreamOutput;
    use runtime_protocol::protocol::ReviewDecision;
    use std::collections::BTreeMap;

    struct StubBackend {
        exec_decisions: Vec<ReviewDecision>,
        patch_decisions: Vec<ReviewDecision>,
        exec_calls: usize,
        patch_calls: usize,
    }

    impl StubBackend {
        fn new(exec_decisions: Vec<ReviewDecision>, patch_decisions: Vec<ReviewDecision>) -> Self {
            Self {
                exec_decisions,
                patch_decisions,
                exec_calls: 0,
                patch_calls: 0,
            }
        }
    }

    impl ApprovalDecisionProvider for StubBackend {
        fn review_exec(&mut self, _event: &ExecApprovalRequestEvent) -> ReviewDecision {
            let decision = self
                .exec_decisions
                .get(self.exec_calls)
                .cloned()
                .unwrap_or(ReviewDecision::Denied);
            self.exec_calls += 1;
            decision
        }

        fn review_patch(&mut self, _event: &ApplyPatchApprovalRequestEvent) -> ReviewDecision {
            let decision = self
                .patch_decisions
                .get(self.patch_calls)
                .cloned()
                .unwrap_or(ReviewDecision::Denied);
            self.patch_calls += 1;
            decision
        }

        fn review_permissions(
            &mut self,
            event: &RequestPermissionsEvent,
        ) -> RequestPermissionsResponse {
            RequestPermissionsResponse {
                permissions: event.permissions.clone(),
                scope: runtime_protocol::request_permissions::PermissionGrantScope::Turn,
                strict_auto_review: false,
            }
        }
    }

    fn runtime_paths() -> ExecServerRuntimePaths {
        ExecServerRuntimePaths::new(std::env::current_exe().unwrap(), None).unwrap()
    }

    async fn context(hooks: HooksToml) -> ApprovalRuntimeContext {
        let cwd = AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap()).unwrap();
        let config = RuntimeConfigBuilder::default()
            .cwd(cwd.clone())
            .load()
            .await
            .unwrap();
        ApprovalRuntimeContext {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            model: "sprite-test".to_string(),
            cwd: cwd.clone(),
            approval_policy: AskForApproval::OnRequest,
            approvals_reviewer: config.approvals_reviewer,
            permission_profile: config.permission_profile,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
            windows_sandbox_private_desktop: true,
            runtime_paths: runtime_paths(),
            sprite_home: std::env::temp_dir().join("sprite-approval-runtime"),
            sprite_linux_sandbox_exe: None,
            hooks,
        }
    }

    fn shell_command(script: &str) -> Vec<String> {
        if cfg!(windows) {
            vec!["cmd".to_string(), "/C".to_string(), script.to_string()]
        } else {
            vec!["sh".to_string(), "-lc".to_string(), script.to_string()]
        }
    }

    fn permission_hook_allow() -> HookHandlerConfig {
        #[cfg(windows)]
        let command_windows = Some(format!(
            "{} {}",
            find_windows_python().display(),
            write_windows_permission_hook_script(
                "allow",
                "import json\nprint(json.dumps({'hookSpecificOutput': {'hookEventName': 'PermissionRequest', 'decision': {'behavior': 'allow'}}}))\n",
            )
            .display()
        ));
        #[cfg(not(windows))]
        let command_windows = None;

        HookHandlerConfig::Command {
            command: "printf '%s' '{\"hookSpecificOutput\":{\"hookEventName\":\"PermissionRequest\",\"decision\":{\"behavior\":\"allow\"}}}'".to_string(),
            command_windows,
            timeout_sec: Some(5),
            r#async: false,
            status_message: Some("running permission request hook".to_string()),
        }
    }

    fn permission_hook_deny() -> HookHandlerConfig {
        HookHandlerConfig::Command {
            command: "echo denied 1>&2; exit 2".to_string(),
            command_windows: Some("echo denied 1>&2 & exit /b 2".to_string()),
            timeout_sec: Some(5),
            r#async: false,
            status_message: None,
        }
    }

    #[cfg(windows)]
    fn find_windows_python() -> PathBuf {
        let output = std::process::Command::new("where.exe")
            .arg("python")
            .output()
            .expect("where python should run");
        assert!(output.status.success(), "where python failed: {:?}", output);
        let stdout = String::from_utf8(output.stdout).expect("where python output should be utf-8");
        stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("python executable should exist")
    }

    #[cfg(windows)]
    fn write_windows_permission_hook_script(stem: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sprite-approval-runtime-{stem}-{}.py",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("hook script should be writable");
        path
    }

    #[tokio::test]
    async fn command_approval_denial_blocks_execution() {
        let mut runtime = ApprovalRuntime::new();
        let ctx = context(HooksToml::default()).await;
        let exec_policy = ExecPolicyManager::default();
        let mut backend = StubBackend::new(vec![ReviewDecision::Denied], Vec::new());

        let outcome = runtime
            .execute_command(
                &ctx,
                &exec_policy,
                &mut backend,
                CommandApprovalRequest {
                    call_id: "call-1".to_string(),
                    command: shell_command("echo should-not-run"),
                    cwd: ctx.cwd.clone(),
                    env: HashMap::new(),
                    env_policy: None,
                    terminal_size: TerminalSize::default(),
                    expiration: ExecExpiration::from(5_000u64),
                    arg0: None,
                    description: None,
                    sandbox_permissions: SandboxPermissions::UseDefault,
                    prefix_rule: None,
                    source: ExecCommandSource::Agent,
                },
            )
            .await
            .expect("command outcome");

        match outcome {
            CommandExecutionOutcome::Declined { events, .. } => {
                assert!(
                    events
                        .iter()
                        .any(|event| matches!(event, EventMsg::ExecApprovalRequest(_)))
                );
            }
            other => panic!("expected declined outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn permission_request_hook_can_allow_command_without_backend_review() {
        let mut hooks = HooksToml::default();
        hooks.events.permission_request.push(config::MatcherGroup {
            matcher: Some("shell".to_string()),
            hooks: vec![permission_hook_allow()],
        });

        let mut runtime = ApprovalRuntime::new();
        let ctx = context(hooks).await;
        let exec_policy = ExecPolicyManager::default();
        let mut backend = StubBackend::new(vec![ReviewDecision::Denied], Vec::new());

        let outcome = runtime
            .execute_command(
                &ctx,
                &exec_policy,
                &mut backend,
                CommandApprovalRequest {
                    call_id: "call-2".to_string(),
                    command: shell_command("echo hook-allowed"),
                    cwd: ctx.cwd.clone(),
                    env: HashMap::new(),
                    env_policy: None,
                    terminal_size: TerminalSize::default(),
                    expiration: ExecExpiration::from(5_000u64),
                    arg0: None,
                    description: Some("hook test".to_string()),
                    sandbox_permissions: SandboxPermissions::UseDefault,
                    prefix_rule: None,
                    source: ExecCommandSource::Agent,
                },
            )
            .await
            .expect("command outcome");

        match outcome {
            CommandExecutionOutcome::Executed { output, .. } => {
                assert!(output.aggregated_output.text.contains("hook-allowed"));
                assert_eq!(backend.exec_calls, 0);
            }
            other => panic!("expected executed outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn permission_request_hook_can_deny_patch_before_user_approval() {
        let mut hooks = HooksToml::default();
        hooks.events.permission_request.push(config::MatcherGroup {
            matcher: Some("apply_patch".to_string()),
            hooks: vec![permission_hook_deny()],
        });

        let mut runtime = ApprovalRuntime::new();
        let ctx = context(hooks).await;
        let mut backend = StubBackend::new(Vec::new(), vec![ReviewDecision::Approved]);

        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("Cargo.toml"),
            FileChange::Update {
                unified_diff: "@@ -1 +1 @@".to_string(),
                move_path: None,
            },
        );
        let mut patch_changes = BTreeMap::new();
        patch_changes.insert(
            PathBuf::from("Cargo.toml"),
            crate::safety::PatchFileChange::Update { move_path: None },
        );

        let outcome = runtime
            .apply_patch(
                &ctx,
                &mut backend,
                PatchApprovalRequest {
                    call_id: "patch-1".to_string(),
                    action: PatchAction::new(patch_changes),
                    changes,
                },
                || async {
                    Ok(ExecToolCallOutput {
                        exit_code: 0,
                        stdout: StreamOutput::new("ok".to_string()),
                        stderr: StreamOutput::new(String::new()),
                        aggregated_output: StreamOutput::new("ok".to_string()),
                        duration: Duration::from_millis(1),
                        timed_out: false,
                    })
                },
            )
            .await
            .expect("patch outcome");

        match outcome {
            PatchExecutionOutcome::Declined { reason, .. } => {
                assert!(reason.contains("denied"));
                assert_eq!(backend.patch_calls, 0);
            }
            other => panic!("expected declined patch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn network_approval_can_persist_session_allowance() {
        let mut runtime = ApprovalRuntime::new();
        let ctx = context(HooksToml::default()).await;
        let exec_policy = ExecPolicyManager::default();
        let mut backend = StubBackend::new(vec![ReviewDecision::ApprovedForSession], Vec::new());
        let request = NetworkApprovalRequest {
            target: "https://example.com".to_string(),
            host: "example.com".to_string(),
            protocol: runtime_protocol::approvals::NetworkApprovalProtocol::Https,
            port: 443,
            reason: Some("retry".to_string()),
        };

        let first = runtime
            .handle_network_access(
                &ctx,
                &exec_policy,
                &mut backend,
                NetworkAccessRequest {
                    call_id: "net-1".to_string(),
                    command: shell_command("curl https://example.com"),
                    cwd: ctx.cwd.clone(),
                    request: request.clone(),
                },
            )
            .await
            .expect("first network result");
        assert!(matches!(
            first,
            NetworkExecutionOutcome::AllowForSession { .. }
        ));

        let second = runtime
            .handle_network_access(
                &ctx,
                &exec_policy,
                &mut backend,
                NetworkAccessRequest {
                    call_id: "net-2".to_string(),
                    command: shell_command("curl https://example.com"),
                    cwd: ctx.cwd.clone(),
                    request,
                },
            )
            .await
            .expect("second network result");
        assert!(matches!(
            second,
            NetworkExecutionOutcome::AllowForSession { .. }
        ));
        assert_eq!(backend.exec_calls, 1);
    }
}
