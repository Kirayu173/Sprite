use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::PermissionProfile;
use runtime_protocol::permissions::FileSystemSandboxPolicy;
use runtime_protocol::protocol::AskForApproval;
use sandboxing::{SandboxType, get_platform_sandbox};
use utils_absolute_path::AbsolutePathBuf;

const PATCH_REJECTED_OUTSIDE_PROJECT_REASON: &str =
    "writing outside of the project; rejected by user approval settings";
const PATCH_REJECTED_READ_ONLY_REASON: &str =
    "writing is blocked by read-only sandbox; rejected by user approval settings";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchFileChange {
    Add,
    Delete,
    Update { move_path: Option<PathBuf> },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchAction {
    changes: BTreeMap<PathBuf, PatchFileChange>,
}

impl PatchAction {
    pub fn new(changes: BTreeMap<PathBuf, PatchFileChange>) -> Self {
        Self { changes }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn changes(&self) -> impl Iterator<Item = (&PathBuf, &PatchFileChange)> {
        self.changes.iter()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SafetyCheck {
    AutoApprove {
        sandbox_type: SandboxType,
        user_explicitly_approved: bool,
    },
    AskUser,
    Reject {
        reason: String,
    },
}

pub fn assess_patch_safety(
    action: &PatchAction,
    policy: AskForApproval,
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
    windows_sandbox_level: WindowsSandboxLevel,
) -> SafetyCheck {
    if action.is_empty() {
        return SafetyCheck::Reject {
            reason: "empty patch".to_string(),
        };
    }

    if matches!(policy, AskForApproval::UnlessTrusted) {
        return SafetyCheck::AskUser;
    }

    let rejects_sandbox_approval = matches!(policy, AskForApproval::Never)
        || matches!(
            policy,
            AskForApproval::Granular(granular) if !granular.sandbox_approval
        );

    if is_write_patch_constrained_to_writable_paths(action, file_system_sandbox_policy, cwd)
        || matches!(policy, AskForApproval::OnFailure)
    {
        if matches!(
            permission_profile,
            PermissionProfile::Disabled | PermissionProfile::External { .. }
        ) {
            SafetyCheck::AutoApprove {
                sandbox_type: SandboxType::None,
                user_explicitly_approved: false,
            }
        } else {
            match get_platform_sandbox(windows_sandbox_level != WindowsSandboxLevel::Disabled) {
                Some(sandbox_type) => SafetyCheck::AutoApprove {
                    sandbox_type,
                    user_explicitly_approved: false,
                },
                None => {
                    if rejects_sandbox_approval {
                        SafetyCheck::Reject {
                            reason: patch_rejection_reason(
                                permission_profile,
                                file_system_sandbox_policy,
                                cwd,
                            )
                            .to_string(),
                        }
                    } else {
                        SafetyCheck::AskUser
                    }
                }
            }
        }
    } else if rejects_sandbox_approval {
        SafetyCheck::Reject {
            reason: patch_rejection_reason(permission_profile, file_system_sandbox_policy, cwd)
                .to_string(),
        }
    } else {
        SafetyCheck::AskUser
    }
}

fn patch_rejection_reason(
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> &'static str {
    match permission_profile {
        PermissionProfile::Managed { .. }
            if !file_system_sandbox_policy.has_full_disk_write_access()
                && file_system_sandbox_policy
                    .get_writable_roots_with_cwd(cwd.as_path())
                    .is_empty() =>
        {
            PATCH_REJECTED_READ_ONLY_REASON
        }
        PermissionProfile::Managed { .. }
        | PermissionProfile::Disabled
        | PermissionProfile::External { .. } => PATCH_REJECTED_OUTSIDE_PROJECT_REASON,
    }
}

fn is_write_patch_constrained_to_writable_paths(
    action: &PatchAction,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> bool {
    let normalize = |path: &Path| -> PathBuf {
        let mut out = PathBuf::new();
        for comp in path.components() {
            match comp {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    };

    let is_path_writable = |p: &Path| {
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p).into_path_buf()
        };
        let abs = normalize(&abs);
        file_system_sandbox_policy.can_write_path_with_cwd(&abs, cwd)
    };

    for (path, change) in action.changes() {
        match change {
            PatchFileChange::Add | PatchFileChange::Delete => {
                if !is_path_writable(path) {
                    return false;
                }
            }
            PatchFileChange::Update { move_path } => {
                if !is_path_writable(path) {
                    return false;
                }
                if let Some(dest) = move_path
                    && !is_path_writable(dest)
                {
                    return false;
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::PermissionProfile;
    use runtime_protocol::permissions::FileSystemAccessMode;
    use runtime_protocol::permissions::FileSystemPath;
    use runtime_protocol::permissions::FileSystemSandboxEntry;
    use runtime_protocol::permissions::FileSystemSandboxPolicy;
    use runtime_protocol::permissions::FileSystemSpecialPath;
    use runtime_protocol::permissions::NetworkSandboxPolicy;
    use runtime_protocol::protocol::GranularApprovalConfig;
    use tempfile::TempDir;

    fn cwd() -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap()).unwrap()
    }

    fn temp_cwd() -> (TempDir, AbsolutePathBuf) {
        let tmp = TempDir::new().unwrap();
        let cwd = AbsolutePathBuf::from_absolute_path(tmp.path()).unwrap();
        (tmp, cwd)
    }

    fn add_action(path: PathBuf) -> PatchAction {
        let mut changes = BTreeMap::new();
        changes.insert(path, PatchFileChange::Add);
        PatchAction::new(changes)
    }

    fn update_action(path: PathBuf) -> PatchAction {
        let mut changes = BTreeMap::new();
        changes.insert(path, PatchFileChange::Update { move_path: None });
        PatchAction::new(changes)
    }

    fn granular(sandbox_approval: bool) -> AskForApproval {
        AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        })
    }

    #[test]
    fn empty_patch_is_rejected() {
        let action = PatchAction::default();
        let result = assess_patch_safety(
            &action,
            AskForApproval::OnRequest,
            &PermissionProfile::read_only(),
            &FileSystemSandboxPolicy::read_only(),
            &cwd(),
            WindowsSandboxLevel::Disabled,
        );
        assert_eq!(
            result,
            SafetyCheck::Reject {
                reason: "empty patch".to_string()
            }
        );
    }

    #[test]
    fn workspace_writable_patch_can_auto_approve() {
        let root = cwd();
        let path = root.join("Cargo.toml").into_path_buf();
        let action = update_action(path);
        let profile = PermissionProfile::workspace_write();
        let fs = profile.file_system_sandbox_policy();
        let result = assess_patch_safety(
            &action,
            AskForApproval::OnRequest,
            &profile,
            &fs,
            &root,
            WindowsSandboxLevel::Disabled,
        );
        assert!(!matches!(result, SafetyCheck::Reject { .. }));
    }

    #[test]
    fn writable_roots_constraint_allows_only_configured_roots() {
        let (_tmp, root) = temp_cwd();
        let parent = AbsolutePathBuf::from_absolute_path(root.as_path().parent().unwrap()).unwrap();
        let inside = add_action(root.join("inner.txt").into_path_buf());
        let outside = add_action(parent.join("outside.txt").into_path_buf());
        let workspace_only = FileSystemSandboxPolicy::workspace_write(
            &[],
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        );

        assert!(is_write_patch_constrained_to_writable_paths(
            &inside,
            &workspace_only,
            &root,
        ));
        assert!(!is_write_patch_constrained_to_writable_paths(
            &outside,
            &workspace_only,
            &root,
        ));

        let with_parent = FileSystemSandboxPolicy::workspace_write(
            std::slice::from_ref(&parent),
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        );
        assert!(is_write_patch_constrained_to_writable_paths(
            &outside,
            &with_parent,
            &root,
        ));
    }

    #[test]
    fn external_sandbox_auto_approves_writable_patch() {
        let (_tmp, root) = temp_cwd();
        let action = add_action(root.join("inner.txt").into_path_buf());
        let profile = PermissionProfile::External {
            network: NetworkSandboxPolicy::Enabled,
        };
        let fs = FileSystemSandboxPolicy::external_sandbox();

        assert_eq!(
            assess_patch_safety(
                &action,
                AskForApproval::OnRequest,
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::AutoApprove {
                sandbox_type: SandboxType::None,
                user_explicitly_approved: false,
            }
        );
    }

    #[test]
    fn granular_allows_user_prompt_for_out_of_root_patch_when_sandbox_approval_enabled() {
        let (_tmp, root) = temp_cwd();
        let parent = AbsolutePathBuf::from_absolute_path(root.as_path().parent().unwrap()).unwrap();
        let action = add_action(parent.join("outside.txt").into_path_buf());
        let profile = PermissionProfile::workspace_write_with(
            &[],
            NetworkSandboxPolicy::Restricted,
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        );
        let fs = profile.file_system_sandbox_policy();

        assert_eq!(
            assess_patch_safety(
                &action,
                AskForApproval::OnRequest,
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::AskUser,
        );
        assert_eq!(
            assess_patch_safety(
                &action,
                granular(/*sandbox_approval*/ true),
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::AskUser,
        );
    }

    #[test]
    fn granular_without_sandbox_approval_rejects_out_of_root_patch() {
        let (_tmp, root) = temp_cwd();
        let parent = AbsolutePathBuf::from_absolute_path(root.as_path().parent().unwrap()).unwrap();
        let action = add_action(parent.join("outside.txt").into_path_buf());
        let profile = PermissionProfile::workspace_write_with(
            &[],
            NetworkSandboxPolicy::Restricted,
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        );
        let fs = profile.file_system_sandbox_policy();

        assert_eq!(
            assess_patch_safety(
                &action,
                granular(/*sandbox_approval*/ false),
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::Reject {
                reason: PATCH_REJECTED_OUTSIDE_PROJECT_REASON.to_string(),
            },
        );
    }

    #[test]
    fn read_only_policy_rejects_patch_with_read_only_reason() {
        let (_tmp, root) = temp_cwd();
        let action = add_action(root.join("inside.txt").into_path_buf());
        let profile = PermissionProfile::read_only();
        let fs = profile.file_system_sandbox_policy();

        assert!(!is_write_patch_constrained_to_writable_paths(
            &action, &fs, &root,
        ));
        assert_eq!(
            assess_patch_safety(
                &action,
                AskForApproval::Never,
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::Reject {
                reason: PATCH_REJECTED_READ_ONLY_REASON.to_string(),
            },
        );
    }

    #[test]
    fn explicit_unreadable_paths_prevent_external_sandbox_auto_approval() {
        let (_tmp, root) = temp_cwd();
        let blocked = root.join("blocked.txt");
        let action = add_action(blocked.clone().into_path_buf());
        let profile = PermissionProfile::External {
            network: NetworkSandboxPolicy::Restricted,
        };
        let fs = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: blocked },
                access: FileSystemAccessMode::Deny,
            },
        ]);

        assert!(!is_write_patch_constrained_to_writable_paths(
            &action, &fs, &root,
        ));
        assert_eq!(
            assess_patch_safety(
                &action,
                AskForApproval::OnRequest,
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::AskUser,
        );
    }

    #[test]
    fn explicit_read_only_subpaths_prevent_external_sandbox_auto_approval() {
        let (_tmp, root) = temp_cwd();
        let docs = root.join("docs");
        let blocked = docs.join("blocked.txt");
        let action = add_action(blocked.into_path_buf());
        let profile = PermissionProfile::External {
            network: NetworkSandboxPolicy::Restricted,
        };
        let fs = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert!(!is_write_patch_constrained_to_writable_paths(
            &action, &fs, &root,
        ));
        assert_eq!(
            assess_patch_safety(
                &action,
                AskForApproval::OnRequest,
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::AskUser,
        );
    }

    #[test]
    fn missing_project_dot_sprite_config_requires_approval() {
        let (_tmp, root) = temp_cwd();
        let config_path = root.join(".sprite").join("config.toml");
        let action = add_action(config_path.into_path_buf());
        let profile = PermissionProfile::workspace_write_with(
            &[],
            NetworkSandboxPolicy::Restricted,
            /*exclude_tmpdir_env_var*/ true,
            /*exclude_slash_tmp*/ true,
        );
        let mut fs = profile.file_system_sandbox_policy();
        fs.entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: root.join(".sprite"),
            },
            access: FileSystemAccessMode::Read,
        });

        assert!(!is_write_patch_constrained_to_writable_paths(
            &action, &fs, &root,
        ));
        assert_eq!(
            assess_patch_safety(
                &action,
                AskForApproval::OnRequest,
                &profile,
                &fs,
                &root,
                WindowsSandboxLevel::Disabled,
            ),
            SafetyCheck::AskUser,
        );
    }
}
