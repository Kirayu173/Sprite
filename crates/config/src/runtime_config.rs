use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::HookEventsToml;
use crate::HooksToml;
use crate::config_toml::ConfigToml;
use crate::config_toml::DEFAULT_PROJECT_DOC_MAX_BYTES;
use crate::loader::load_config_layers_state;
use crate::permissions::ResolvedPermissionProfile;
use crate::permissions::resolve_effective_permission_profile;
use crate::state::ConfigLayerStack;
use crate::state::ConfigLoadOptions;
use crate::state::LoaderOverrides;
use crate::thread_config::NoopThreadConfigLoader;
use crate::thread_config::ThreadConfigLoader;
use crate::types::ApprovalsReviewer;
use crate::types::MemoriesConfig;
use crate::types::OAuthCredentialsStoreMode;
use crate::types::SkillsConfig;
use crate::types::WindowsToml;
use file_system::ExecutorFileSystem;
use model_provider_info::LEGACY_OLLAMA_CHAT_PROVIDER_ID;
use model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use model_provider_info::ModelProviderInfo;
use model_provider_info::OLLAMA_CHAT_PROVIDER_REMOVED_ERROR;
use model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use model_provider_info::OPENAI_COMPATIBLE_PROVIDER_ID;
use model_provider_info::built_in_model_providers;
use model_provider_info::merge_configured_model_providers;
use runtime_protocol::config_types::AutoCompactTokenLimitScope;
use runtime_protocol::config_types::SandboxMode;
use runtime_protocol::config_types::ShellEnvironmentPolicy;
use runtime_protocol::config_types::WebSearchMode;
use runtime_protocol::config_types::WebSearchToolConfig;
use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::ActivePermissionProfile;
use runtime_protocol::models::PermissionProfile;
use runtime_protocol::protocol::AskForApproval;
use toml::Value as TomlValue;
use utils_absolute_path::AbsolutePathBuf;
use utils_absolute_path::AbsolutePathBufGuard;

pub const DEFAULT_MODEL: &str = "gpt-oss";
pub const DEFAULT_MODEL_PROVIDER_ID: &str = OLLAMA_OSS_PROVIDER_ID;

#[derive(Debug, Default)]
struct NativeConfigFileSystem;

impl ExecutorFileSystem for NativeConfigFileSystem {}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfig {
    pub raw: ConfigToml,
    pub config_layer_stack: ConfigLayerStack,
    pub model: String,
    pub review_model: Option<String>,
    pub model_context_window: Option<i64>,
    pub model_auto_compact_token_limit: Option<i64>,
    pub model_auto_compact_token_limit_scope: AutoCompactTokenLimitScope,
    pub model_provider_id: String,
    pub model_provider: ModelProviderInfo,
    pub model_providers: HashMap<String, ModelProviderInfo>,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub permission_profile: PermissionProfile,
    pub active_permission_profile: ActivePermissionProfile,
    pub permission_profile_workspace_roots: Vec<AbsolutePathBuf>,
    pub permission_warnings: Vec<String>,
    pub shell_environment_policy: ShellEnvironmentPolicy,
    pub allow_login_shell: bool,
    pub mcp_servers: HashMap<String, crate::McpServerConfig>,
    pub mcp_oauth_credentials_store: OAuthCredentialsStoreMode,
    pub mcp_oauth_callback_port: Option<u16>,
    pub mcp_oauth_callback_url: Option<String>,
    pub skills: SkillsConfig,
    pub hooks: HooksToml,
    pub windows: WindowsToml,
    pub memories: MemoriesConfig,
    pub project_doc_max_bytes: usize,
    pub project_doc_fallback_filenames: Vec<String>,
    pub tool_output_token_limit: Option<usize>,
    pub log_dir: PathBuf,
    pub sqlite_home: PathBuf,
    pub web_search: WebSearchMode,
    pub web_search_tool_config: Option<WebSearchToolConfig>,
    pub hide_agent_reasoning: bool,
    pub show_raw_agent_reasoning: bool,
    pub include_permissions_instructions: bool,
    pub include_apps_instructions: bool,
    pub include_collaboration_mode_instructions: bool,
    pub include_environment_context: bool,
    pub cwd: AbsolutePathBuf,
    pub sprite_home: AbsolutePathBuf,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfigBuilder {
    sprite_home: Option<PathBuf>,
    cwd: Option<AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    load_options: ConfigLoadOptions,
    model_provider_override: Option<String>,
}

impl Default for RuntimeConfigBuilder {
    fn default() -> Self {
        Self {
            sprite_home: None,
            cwd: None,
            cli_overrides: Vec::new(),
            load_options: ConfigLoadOptions::default(),
            model_provider_override: None,
        }
    }
}

impl RuntimeConfigBuilder {
    pub fn sprite_home(mut self, sprite_home: impl Into<PathBuf>) -> Self {
        self.sprite_home = Some(sprite_home.into());
        self
    }

    pub fn cwd(mut self, cwd: AbsolutePathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    pub fn cli_overrides(mut self, cli_overrides: Vec<(String, TomlValue)>) -> Self {
        self.cli_overrides = cli_overrides;
        self
    }

    pub fn load_options(mut self, load_options: ConfigLoadOptions) -> Self {
        self.load_options = load_options;
        self
    }

    pub fn loader_overrides(mut self, loader_overrides: LoaderOverrides) -> Self {
        self.load_options.loader_overrides = loader_overrides;
        self
    }

    pub fn strict_config(mut self, strict_config: bool) -> Self {
        self.load_options.strict_config = strict_config;
        self
    }

    pub fn model_provider_override(mut self, model_provider: impl Into<String>) -> Self {
        self.model_provider_override = Some(model_provider.into());
        self
    }

    pub async fn load(self) -> io::Result<RuntimeConfig> {
        let fs = NativeConfigFileSystem;
        self.load_with(&fs, &NoopThreadConfigLoader).await
    }

    pub async fn load_with(
        self,
        fs: &dyn ExecutorFileSystem,
        thread_config_loader: &dyn ThreadConfigLoader,
    ) -> io::Result<RuntimeConfig> {
        let sprite_home = self.resolve_sprite_home()?;
        let cwd = self.resolve_cwd()?;
        let stack = load_config_layers_state(
            fs,
            &sprite_home,
            Some(cwd.clone()),
            &self.cli_overrides,
            self.load_options,
            thread_config_loader,
        )
        .await?;

        RuntimeConfig::from_layer_stack(
            stack,
            sprite_home,
            cwd,
            self.model_provider_override.as_deref(),
        )
    }

    fn resolve_sprite_home(&self) -> io::Result<PathBuf> {
        if let Some(sprite_home) = self.sprite_home.clone() {
            return Ok(sprite_home);
        }
        if let Some(value) = std::env::var_os("SPRITE_HOME") {
            return Ok(PathBuf::from(value));
        }
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "SPRITE_HOME is unset and no home directory environment variable is available",
                )
            })?;
        Ok(PathBuf::from(home).join(".sprite"))
    }

    fn resolve_cwd(&self) -> io::Result<AbsolutePathBuf> {
        match self.cwd.clone() {
            Some(cwd) => Ok(cwd),
            None => AbsolutePathBuf::current_dir(),
        }
    }
}

impl RuntimeConfig {
    pub fn builder() -> RuntimeConfigBuilder {
        RuntimeConfigBuilder::default()
    }

    pub fn from_layer_stack(
        config_layer_stack: ConfigLayerStack,
        sprite_home: impl AsRef<Path>,
        cwd: AbsolutePathBuf,
        model_provider_override: Option<&str>,
    ) -> io::Result<Self> {
        let effective_config = config_layer_stack.effective_config();
        let _guard = AbsolutePathBufGuard::new(cwd.as_path());
        let raw: ConfigToml = effective_config
            .try_into()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        drop(_guard);

        let sprite_home = AbsolutePathBuf::from_absolute_path(sprite_home.as_ref())?;
        Self::from_toml(
            raw,
            config_layer_stack,
            sprite_home,
            cwd,
            model_provider_override,
        )
    }

    fn from_toml(
        raw: ConfigToml,
        config_layer_stack: ConfigLayerStack,
        sprite_home: AbsolutePathBuf,
        cwd: AbsolutePathBuf,
        model_provider_override: Option<&str>,
    ) -> io::Result<Self> {
        let model_providers = merge_configured_model_providers(
            built_in_model_providers(raw.provider_base_url.clone()),
            raw.model_providers.clone(),
        )
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;

        let model_provider_id = select_model_provider_id(&raw, model_provider_override);
        let model_provider = model_providers
            .get(&model_provider_id)
            .cloned()
            .ok_or_else(|| {
                let message = if model_provider_id == LEGACY_OLLAMA_CHAT_PROVIDER_ID {
                    OLLAMA_CHAT_PROVIDER_REMOVED_ERROR.to_string()
                } else {
                    format!("Model provider `{model_provider_id}` not found")
                };
                io::Error::new(io::ErrorKind::InvalidInput, message)
            })?;

        let active_project = raw.get_active_project(cwd.as_path(), /*repo_root*/ None);
        let windows_sandbox_level = windows_sandbox_level(&raw.windows);
        let ResolvedPermissionProfile {
            profile: permission_profile,
            active_profile: active_permission_profile,
            workspace_roots: permission_profile_workspace_roots,
            warnings: permission_warnings,
        } = if let Some(default_permissions) = raw.default_permissions.as_deref() {
            resolve_effective_permission_profile(
                raw.permissions.as_ref(),
                Some(default_permissions),
                raw.sandbox_workspace_write.as_ref(),
                active_project.as_ref(),
                windows_sandbox_level,
                cwd.as_path(),
            )?
        } else {
            let permission_profile = futures::executor::block_on(raw.derive_permission_profile(
                raw.sandbox_mode,
                windows_sandbox_level,
                active_project.as_ref(),
                Some(&config_layer_stack.requirements().permission_profile.value),
            ));
            ResolvedPermissionProfile {
                profile: permission_profile,
                active_profile: ActivePermissionProfile::new(
                    raw.sandbox_mode
                        .map(legacy_sandbox_profile_name)
                        .unwrap_or_else(|| {
                            crate::permissions::default_builtin_permission_profile_name(
                                active_project.as_ref(),
                                windows_sandbox_level,
                            )
                        }),
                ),
                workspace_roots: Vec::new(),
                warnings: Vec::new(),
            }
        };

        Ok(Self {
            model: raw
                .model
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            review_model: raw.review_model.clone(),
            model_context_window: raw.model_context_window,
            model_auto_compact_token_limit: raw.model_auto_compact_token_limit,
            model_auto_compact_token_limit_scope: raw
                .model_auto_compact_token_limit_scope
                .unwrap_or_default(),
            model_provider_id,
            model_provider,
            model_providers,
            approval_policy: raw.approval_policy.unwrap_or_default(),
            approvals_reviewer: raw.approvals_reviewer.unwrap_or_default(),
            permission_profile,
            active_permission_profile,
            permission_profile_workspace_roots,
            permission_warnings,
            shell_environment_policy: ShellEnvironmentPolicy::from(
                raw.shell_environment_policy.clone(),
            ),
            allow_login_shell: raw.allow_login_shell.unwrap_or(true),
            mcp_servers: raw.mcp_servers.clone(),
            mcp_oauth_credentials_store: raw.mcp_oauth_credentials_store.unwrap_or_default(),
            mcp_oauth_callback_port: raw.mcp_oauth_callback_port,
            mcp_oauth_callback_url: raw.mcp_oauth_callback_url.clone(),
            skills: raw.skills.clone().unwrap_or_default(),
            hooks: raw.hooks.clone().unwrap_or_else(|| HooksToml {
                events: HookEventsToml::default(),
                state: Default::default(),
            }),
            windows: raw.windows.clone().unwrap_or_default(),
            memories: raw.memories.clone().map(Into::into).unwrap_or_default(),
            project_doc_max_bytes: raw
                .project_doc_max_bytes
                .unwrap_or(DEFAULT_PROJECT_DOC_MAX_BYTES),
            project_doc_fallback_filenames: raw
                .project_doc_fallback_filenames
                .clone()
                .unwrap_or_default(),
            tool_output_token_limit: raw.tool_output_token_limit,
            log_dir: raw
                .log_dir
                .clone()
                .map(AbsolutePathBuf::into_path_buf)
                .unwrap_or_else(|| sprite_home.join("log").into_path_buf()),
            sqlite_home: raw
                .sqlite_home
                .clone()
                .map(AbsolutePathBuf::into_path_buf)
                .unwrap_or_else(|| sprite_home.clone().into_path_buf()),
            web_search: raw.web_search.unwrap_or_default(),
            web_search_tool_config: raw
                .tools
                .as_ref()
                .and_then(|tools| tools.web_search.clone()),
            hide_agent_reasoning: raw.hide_agent_reasoning.unwrap_or(false),
            show_raw_agent_reasoning: raw.show_raw_agent_reasoning.unwrap_or(false),
            include_permissions_instructions: raw.include_permissions_instructions.unwrap_or(true),
            include_apps_instructions: raw.include_apps_instructions.unwrap_or(true),
            include_collaboration_mode_instructions: raw
                .include_collaboration_mode_instructions
                .unwrap_or(true),
            include_environment_context: raw.include_environment_context.unwrap_or(true),
            raw,
            config_layer_stack,
            cwd,
            sprite_home,
        })
    }
}

fn select_model_provider_id(config: &ConfigToml, model_provider_override: Option<&str>) -> String {
    if let Some(model_provider_override) = model_provider_override {
        return model_provider_override.to_string();
    }
    if let Some(model_provider) = config.model_provider.clone() {
        return model_provider;
    }
    if let Some(oss_provider) = config.oss_provider.as_deref() {
        return match oss_provider {
            OLLAMA_OSS_PROVIDER_ID | LMSTUDIO_OSS_PROVIDER_ID => oss_provider.to_string(),
            _ => OPENAI_COMPATIBLE_PROVIDER_ID.to_string(),
        };
    }
    DEFAULT_MODEL_PROVIDER_ID.to_string()
}

fn windows_sandbox_level(windows: &Option<WindowsToml>) -> WindowsSandboxLevel {
    match windows.as_ref().and_then(|windows| windows.sandbox) {
        Some(crate::types::WindowsSandboxModeToml::Elevated) => WindowsSandboxLevel::Elevated,
        Some(crate::types::WindowsSandboxModeToml::Unelevated) => {
            WindowsSandboxLevel::RestrictedToken
        }
        None => WindowsSandboxLevel::Disabled,
    }
}

fn legacy_sandbox_profile_name(sandbox_mode: SandboxMode) -> &'static str {
    match sandbox_mode {
        SandboxMode::ReadOnly => crate::permissions::BUILT_IN_READ_ONLY_PROFILE,
        SandboxMode::WorkspaceWrite => crate::permissions::BUILT_IN_WORKSPACE_PROFILE,
        SandboxMode::DangerFullAccess => crate::permissions::BUILT_IN_DANGER_FULL_ACCESS_PROFILE,
    }
}

#[cfg(test)]
#[path = "runtime_config_tests.rs"]
mod tests;
