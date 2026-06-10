use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use config::types::WindowsSandboxModeToml;
use config::{ConfigEditsBuilder, RuntimeConfig};
use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::PermissionProfile;
use toml_edit::value;
use utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsSandboxSetupMode {
    Elevated,
    Unelevated,
}

#[derive(Debug, Clone)]
pub struct WindowsSandboxSetupRequest {
    pub mode: WindowsSandboxSetupMode,
    pub permission_profile: PermissionProfile,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub command_cwd: PathBuf,
    pub env_map: HashMap<String, String>,
    pub sprite_home: PathBuf,
}

pub fn windows_sandbox_level_from_runtime_config(config: &RuntimeConfig) -> WindowsSandboxLevel {
    match config.windows.sandbox {
        Some(WindowsSandboxModeToml::Elevated) => WindowsSandboxLevel::Elevated,
        Some(WindowsSandboxModeToml::Unelevated) => WindowsSandboxLevel::RestrictedToken,
        None => WindowsSandboxLevel::Disabled,
    }
}

pub fn resolve_windows_sandbox_private_desktop(config: &RuntimeConfig) -> bool {
    config.windows.sandbox_private_desktop.unwrap_or(true)
}

#[cfg(target_os = "windows")]
pub fn sandbox_setup_is_complete(sprite_home: &Path) -> bool {
    windows_sandbox::sandbox_setup_is_complete(sprite_home)
}

#[cfg(not(target_os = "windows"))]
pub fn sandbox_setup_is_complete(_sprite_home: &Path) -> bool {
    false
}

#[cfg(target_os = "windows")]
pub fn run_elevated_setup(
    permission_profile: &PermissionProfile,
    workspace_roots: &[AbsolutePathBuf],
    command_cwd: &Path,
    env_map: &HashMap<String, String>,
    sprite_home: &Path,
) -> Result<()> {
    let permissions =
        windows_sandbox::ResolvedWindowsSandboxPermissions::try_from_permission_profile_for_workspace_roots(
            permission_profile,
            workspace_roots,
        )?;
    windows_sandbox::run_elevated_setup(
        windows_sandbox::SandboxSetupRequest {
            permissions: &permissions,
            command_cwd,
            env_map,
            sprite_home: sprite_home,
            proxy_enforced: false,
        },
        windows_sandbox::SetupRootOverrides::default(),
    )
}

#[cfg(not(target_os = "windows"))]
pub fn run_elevated_setup(
    _permission_profile: &PermissionProfile,
    _workspace_roots: &[AbsolutePathBuf],
    _command_cwd: &Path,
    _env_map: &HashMap<String, String>,
    _sprite_home: &Path,
) -> Result<()> {
    anyhow::bail!("elevated Windows sandbox setup is only supported on Windows")
}

#[cfg(target_os = "windows")]
pub fn run_legacy_setup_preflight(
    permission_profile: &PermissionProfile,
    workspace_roots: &[AbsolutePathBuf],
    command_cwd: &Path,
    env_map: &HashMap<String, String>,
    sprite_home: &Path,
) -> Result<()> {
    windows_sandbox::run_windows_sandbox_legacy_preflight(
        permission_profile,
        workspace_roots,
        sprite_home,
        command_cwd,
        env_map,
    )
}

#[cfg(not(target_os = "windows"))]
pub fn run_legacy_setup_preflight(
    _permission_profile: &PermissionProfile,
    _workspace_roots: &[AbsolutePathBuf],
    _command_cwd: &Path,
    _env_map: &HashMap<String, String>,
    _sprite_home: &Path,
) -> Result<()> {
    anyhow::bail!("legacy Windows sandbox setup is only supported on Windows")
}

#[cfg(target_os = "windows")]
pub fn run_setup_refresh_with_extra_read_roots(
    permission_profile: &PermissionProfile,
    workspace_roots: &[AbsolutePathBuf],
    command_cwd: &Path,
    env_map: &HashMap<String, String>,
    sprite_home: &Path,
    extra_read_roots: Vec<PathBuf>,
) -> Result<()> {
    windows_sandbox::run_setup_refresh_with_extra_read_roots(
        permission_profile,
        workspace_roots,
        command_cwd,
        env_map,
        sprite_home,
        extra_read_roots,
        false,
    )
}

#[cfg(not(target_os = "windows"))]
pub fn run_setup_refresh_with_extra_read_roots(
    _permission_profile: &PermissionProfile,
    _workspace_roots: &[AbsolutePathBuf],
    _command_cwd: &Path,
    _env_map: &HashMap<String, String>,
    _sprite_home: &Path,
    _extra_read_roots: Vec<PathBuf>,
) -> Result<()> {
    anyhow::bail!("Windows sandbox read-root refresh is only supported on Windows")
}

pub async fn run_windows_sandbox_setup(request: WindowsSandboxSetupRequest) -> Result<()> {
    let mode = request.mode;
    let permission_profile = request.permission_profile;
    let workspace_roots = request.workspace_roots;
    let command_cwd = request.command_cwd;
    let env_map = request.env_map;
    let sprite_home = request.sprite_home;
    let setup_home = sprite_home.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        match mode {
            WindowsSandboxSetupMode::Elevated => {
                if !sandbox_setup_is_complete(setup_home.as_path()) {
                    run_elevated_setup(
                        &permission_profile,
                        workspace_roots.as_slice(),
                        command_cwd.as_path(),
                        &env_map,
                        setup_home.as_path(),
                    )?;
                }
            }
            WindowsSandboxSetupMode::Unelevated => {
                run_legacy_setup_preflight(
                    &permission_profile,
                    workspace_roots.as_slice(),
                    command_cwd.as_path(),
                    &env_map,
                    setup_home.as_path(),
                )?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|join_err| anyhow::anyhow!("windows sandbox setup task failed: {join_err}"))??;

    persist_windows_sandbox_mode(sprite_home.as_path(), mode)
        .await
        .map_err(|err| anyhow::anyhow!("failed to persist windows sandbox mode: {err}"))
}

async fn persist_windows_sandbox_mode(
    sprite_home: &Path,
    mode: WindowsSandboxSetupMode,
) -> std::io::Result<()> {
    ConfigEditsBuilder::new(sprite_home)
        .set_path(
            vec!["windows".to_string(), "sandbox".to_string()],
            value(match mode {
                WindowsSandboxSetupMode::Elevated => "elevated",
                WindowsSandboxSetupMode::Unelevated => "unelevated",
            }),
        )
        .apply()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::RuntimeConfigBuilder;

    #[tokio::test]
    async fn runtime_config_windows_flags_map_to_level() {
        let cwd = AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap()).unwrap();
        let mut config = RuntimeConfigBuilder::default()
            .cwd(cwd)
            .load()
            .await
            .unwrap();
        config.windows.sandbox = Some(WindowsSandboxModeToml::Elevated);
        assert_eq!(
            windows_sandbox_level_from_runtime_config(&config),
            WindowsSandboxLevel::Elevated
        );
    }
}
