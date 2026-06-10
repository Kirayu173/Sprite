use std::path::Path;

use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::PermissionProfile;
use sandboxing::{
    SandboxType, get_platform_sandbox, policy_transforms::should_require_platform_sandbox,
};

pub fn permission_profile_sandbox_tag(
    profile: &PermissionProfile,
    windows_sandbox_level: WindowsSandboxLevel,
    enforce_managed_network: bool,
) -> &'static str {
    match profile {
        PermissionProfile::Disabled => return "none",
        PermissionProfile::External { .. } => return "external",
        PermissionProfile::Managed {
            file_system,
            network,
        } => {
            let file_system_policy = file_system.to_sandbox_policy();
            if !should_require_platform_sandbox(
                &file_system_policy,
                *network,
                enforce_managed_network,
            ) {
                return "none";
            }
        }
    }
    if cfg!(target_os = "windows") && matches!(windows_sandbox_level, WindowsSandboxLevel::Elevated)
    {
        return "windows_elevated";
    }

    get_platform_sandbox(windows_sandbox_level != WindowsSandboxLevel::Disabled)
        .map(SandboxType::as_metric_tag)
        .unwrap_or("none")
}

pub fn permission_profile_policy_tag(profile: &PermissionProfile, cwd: &Path) -> &'static str {
    match profile {
        PermissionProfile::Disabled => "danger-full-access",
        PermissionProfile::External { .. } => "external-sandbox",
        PermissionProfile::Managed { .. } => {
            let file_system_policy = profile.file_system_sandbox_policy();
            if file_system_policy.has_full_disk_write_access() {
                "danger-full-access"
            } else if file_system_policy
                .get_writable_roots_with_cwd(cwd)
                .is_empty()
            {
                "read-only"
            } else {
                "workspace-write"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::models::PermissionProfile;

    #[test]
    fn disabled_profile_uses_none_and_full_access_tags() {
        let profile = PermissionProfile::Disabled;
        assert_eq!(
            permission_profile_sandbox_tag(&profile, WindowsSandboxLevel::Disabled, false),
            "none"
        );
        assert_eq!(
            permission_profile_policy_tag(&profile, std::env::current_dir().unwrap().as_path()),
            "danger-full-access"
        );
    }
}
