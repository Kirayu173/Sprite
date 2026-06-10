use std::collections::HashMap;

use network_proxy::NetworkProxy;
use runtime_protocol::approvals::{ExecPolicyAmendment, NetworkApprovalContext};
use runtime_protocol::models::{PermissionProfile, SandboxPermissions};
use runtime_protocol::permissions::{FileSystemSandboxKind, FileSystemSandboxPolicy};
use runtime_protocol::protocol::{AskForApproval, ReviewDecision};
use sandboxing::{
    SandboxCommand, SandboxExecRequest, SandboxManager, SandboxTransformError,
    SandboxTransformRequest, SandboxType, SandboxablePreference,
};

#[derive(Clone, Default, Debug)]
pub struct ApprovalStore {
    map: HashMap<String, ReviewDecision>,
}

impl ApprovalStore {
    pub fn get<K: serde::Serialize>(&self, key: &K) -> Option<ReviewDecision> {
        let serialized = serde_json::to_string(key).ok()?;
        self.map.get(&serialized).cloned()
    }

    pub fn put<K: serde::Serialize>(&mut self, key: K, value: ReviewDecision) {
        if let Ok(serialized) = serde_json::to_string(&key) {
            self.map.insert(serialized, value);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecApprovalRequirement {
    Skip {
        bypass_sandbox: bool,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    NeedsApproval {
        reason: Option<String>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    Forbidden {
        reason: String,
    },
}

impl ExecApprovalRequirement {
    pub fn proposed_execpolicy_amendment(&self) -> Option<&ExecPolicyAmendment> {
        match self {
            Self::Skip {
                proposed_execpolicy_amendment: Some(amendment),
                ..
            }
            | Self::NeedsApproval {
                proposed_execpolicy_amendment: Some(amendment),
                ..
            } => Some(amendment),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxOverride {
    NoOverride,
    BypassSandboxFirstAttempt,
}

#[derive(Debug, Clone)]
pub struct PermissionRequestPayload {
    pub command: String,
    pub description: Option<String>,
    pub network_approval_context: Option<NetworkApprovalContext>,
}

pub fn default_exec_approval_requirement(
    policy: AskForApproval,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> ExecApprovalRequirement {
    let needs_approval = match policy {
        AskForApproval::Never | AskForApproval::OnFailure => false,
        AskForApproval::OnRequest | AskForApproval::Granular(_) => {
            matches!(
                file_system_sandbox_policy.kind,
                FileSystemSandboxKind::Restricted
            )
        }
        AskForApproval::UnlessTrusted => true,
    };

    if needs_approval
        && matches!(
            policy,
            AskForApproval::Granular(granular) if !granular.allows_sandbox_approval()
        )
    {
        ExecApprovalRequirement::Forbidden {
            reason: "approval policy disallowed sandbox approval prompt".to_string(),
        }
    } else if needs_approval {
        ExecApprovalRequirement::NeedsApproval {
            reason: None,
            proposed_execpolicy_amendment: None,
        }
    } else {
        ExecApprovalRequirement::Skip {
            bypass_sandbox: false,
            proposed_execpolicy_amendment: None,
        }
    }
}

pub fn unsandboxed_execution_allowed(file_system_sandbox_policy: &FileSystemSandboxPolicy) -> bool {
    !file_system_sandbox_policy.has_denied_read_restrictions()
}

pub fn sandbox_override_for_first_attempt(
    sandbox_permissions: SandboxPermissions,
    exec_approval_requirement: &ExecApprovalRequirement,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> SandboxOverride {
    if !unsandboxed_execution_allowed(file_system_sandbox_policy) {
        return SandboxOverride::NoOverride;
    }

    if matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip {
            bypass_sandbox: true,
            ..
        }
    ) {
        return SandboxOverride::BypassSandboxFirstAttempt;
    }

    if sandbox_permissions.requires_escalated_permissions() {
        SandboxOverride::BypassSandboxFirstAttempt
    } else {
        SandboxOverride::NoOverride
    }
}

pub fn sandbox_permissions_preserving_denied_reads(
    sandbox_permissions: SandboxPermissions,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> SandboxPermissions {
    if sandbox_permissions.requires_escalated_permissions()
        && !unsandboxed_execution_allowed(file_system_sandbox_policy)
    {
        SandboxPermissions::UseDefault
    } else {
        sandbox_permissions
    }
}

pub fn managed_network_for_sandbox_permissions<'a>(
    network: Option<&'a NetworkProxy>,
    sandbox_permissions: SandboxPermissions,
) -> Option<&'a NetworkProxy> {
    if sandbox_permissions.requires_escalated_permissions() {
        None
    } else {
        network
    }
}

pub fn select_initial_sandbox(
    manager: &SandboxManager,
    permission_profile: &PermissionProfile,
    preference: SandboxablePreference,
    windows_sandbox_level: runtime_protocol::config_types::WindowsSandboxLevel,
    managed_network: Option<&NetworkProxy>,
) -> SandboxType {
    let (file_system_policy, network_policy) = permission_profile.to_runtime_permissions();
    manager.select_initial(
        &file_system_policy,
        network_policy,
        preference,
        windows_sandbox_level,
        managed_network.is_some(),
    )
}

pub fn build_sandbox_exec_request(
    manager: &SandboxManager,
    command: SandboxCommand,
    permissions: &PermissionProfile,
    sandbox: SandboxType,
    enforce_managed_network: bool,
    network: Option<&NetworkProxy>,
    sandbox_policy_cwd: &std::path::Path,
    sprite_linux_sandbox_exe: Option<&std::path::Path>,
    windows_sandbox_level: runtime_protocol::config_types::WindowsSandboxLevel,
    windows_sandbox_private_desktop: bool,
) -> Result<SandboxExecRequest, SandboxTransformError> {
    manager.transform(SandboxTransformRequest {
        command,
        permissions,
        sandbox,
        enforce_managed_network,
        network,
        sandbox_policy_cwd,
        sprite_linux_sandbox_exe,
        use_legacy_landlock: false,
        windows_sandbox_level,
        windows_sandbox_private_desktop,
    })
}

pub fn permission_request_payload(
    command: String,
    description: Option<String>,
    network_approval_context: Option<NetworkApprovalContext>,
) -> PermissionRequestPayload {
    PermissionRequestPayload {
        command,
        description,
        network_approval_context,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use utils_absolute_path::AbsolutePathBuf;

    #[test]
    fn denied_reads_disable_unsandboxed_override() {
        let profile = PermissionProfile::read_only();
        let mut file_system = profile.file_system_sandbox_policy();
        file_system
            .entries
            .push(runtime_protocol::permissions::FileSystemSandboxEntry {
                path: runtime_protocol::permissions::FileSystemPath::Path {
                    path: AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap())
                        .unwrap(),
                },
                access: runtime_protocol::permissions::FileSystemAccessMode::Deny,
            });
        assert_eq!(
            sandbox_permissions_preserving_denied_reads(
                SandboxPermissions::RequireEscalated,
                &file_system,
            ),
            SandboxPermissions::UseDefault
        );
    }

    #[test]
    fn default_requirement_rejects_disallowed_granular_prompt() {
        let result = default_exec_approval_requirement(
            AskForApproval::Granular(runtime_protocol::protocol::GranularApprovalConfig {
                rules: true,
                sandbox_approval: false,
                skill_approval: true,
                request_permissions: true,
                mcp_elicitations: true,
            }),
            &PermissionProfile::read_only().file_system_sandbox_policy(),
        );
        assert!(matches!(result, ExecApprovalRequirement::Forbidden { .. }));
    }
}
