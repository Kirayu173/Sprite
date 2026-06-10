use std::collections::BTreeMap;
use std::collections::HashMap;

use file_system::ExecutorFileSystem;
use model_provider_info::ModelProviderInfo;
use model_provider_info::WireApi;
use pretty_assertions::assert_eq;
use runtime_protocol::models::BUILT_IN_PERMISSION_PROFILE_READ_ONLY;
use runtime_protocol::models::BUILT_IN_PERMISSION_PROFILE_WORKSPACE;
use runtime_protocol::permissions::FileSystemAccessMode;
use runtime_protocol::permissions::FileSystemPath;
use runtime_protocol::permissions::FileSystemSandboxKind;
use runtime_protocol::permissions::NetworkSandboxPolicy;
use runtime_protocol::protocol::HookEventName;
use tempfile::TempDir;
use tokio::fs;

use super::*;
use crate::ConfigLoadOptions;
use crate::LoaderOverrides;
use crate::McpServerDisabledReason;
use crate::RequirementSource;
use crate::SessionThreadConfig;
use crate::StaticThreadConfigLoader;
use crate::ThreadConfigSource;
use crate::UserThreadConfig;
use utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Default)]
struct TestFileSystem;

impl ExecutorFileSystem for TestFileSystem {}

#[tokio::test]
async fn default_runtime_config_loads_without_user_config_or_auth() {
    let temp_dir = TempDir::new().expect("tempdir");
    let sprite_home = temp_dir.path().join("home");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd.clone())
        .loader_overrides(LoaderOverrides::without_host_requirements_for_tests())
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect("default config loads");

    assert_eq!(config.model, DEFAULT_MODEL);
    assert_eq!(config.model_provider_id, DEFAULT_MODEL_PROVIDER_ID);
    assert_eq!(config.cwd, cwd);
    assert_eq!(
        config.active_permission_profile.id,
        BUILT_IN_PERMISSION_PROFILE_READ_ONLY
    );
    assert!(config.mcp_servers.is_empty());
    assert!(config.skills.config.is_empty());
}

#[tokio::test]
async fn runtime_config_uses_sqlite_home_environment_default() {
    let temp_dir = TempDir::new().expect("tempdir");
    let sprite_home = temp_dir.path().join("home");
    let sqlite_home = temp_dir.path().join("sqlite");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
    let _env_guard = EnvVarGuard::set("SPRITE_SQLITE_HOME", &sqlite_home);

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd)
        .loader_overrides(LoaderOverrides::without_host_requirements_for_tests())
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect("runtime config loads");

    assert_eq!(config.sqlite_home, sqlite_home);
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[tokio::test]
async fn runtime_config_merges_provider_permission_mcp_skills_and_hooks() {
    let temp_dir = TempDir::new().expect("tempdir");
    let sprite_home = temp_dir.path().join("home");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
    let config_toml = sprite_home.join(crate::CONFIG_TOML_FILE);
    fs::write(
        &config_toml,
        r#"
model = "custom-model"
model_provider = "local"
default_permissions = "locked"

[model_providers.local]
name = "local"
base_url = "http://127.0.0.1:8061/v1"
wire_api = "responses"

[permissions.locked.filesystem]
":root" = "read"
":workspace_roots" = { "private/**/*.env" = "deny" }

[permissions.locked.network]
enabled = true

[mcp_servers.local]
command = "node"
args = ["server.js"]

[skills]
include_instructions = false

[[skills.config]]
name = "reviewer"
enabled = true

[hooks]
PreToolUse = [{ matcher = "shell", hooks = [{ type = "command", command = "echo ok" }] }]
"#,
    )
    .await
    .expect("write config");

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd)
        .loader_overrides(LoaderOverrides::without_host_requirements_for_tests())
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect("runtime config loads");

    assert_eq!(config.model, "custom-model");
    assert_eq!(config.model_provider_id, "local");
    assert_eq!(
        config.model_provider.base_url.as_deref(),
        Some("http://127.0.0.1:8061/v1")
    );
    assert_eq!(config.active_permission_profile.id, "locked");
    assert_eq!(
        config.permission_profile.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
    assert_eq!(
        config.permission_profile.file_system_sandbox_policy().kind,
        FileSystemSandboxKind::Restricted
    );
    assert!(
        config
            .permission_profile
            .file_system_sandbox_policy()
            .entries
            .iter()
            .any(|entry| matches!(
                (&entry.path, entry.access),
                (FileSystemPath::Special { .. }, FileSystemAccessMode::Read)
            ))
    );
    assert!(config.mcp_servers.contains_key("local"));
    assert_eq!(
        config.skills.include_instructions,
        Some(false),
        "skills config should be parsed into the typed runtime view"
    );
    assert_eq!(config.hooks.events.handler_count(), 1);
    let event_names = config
        .hooks
        .events
        .clone()
        .into_matcher_groups()
        .into_iter()
        .filter(|(_, groups)| !groups.is_empty())
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    assert_eq!(event_names, vec![HookEventName::PreToolUse]);
}

#[tokio::test]
async fn runtime_config_prefers_local_provider_default_model_when_model_is_unset() {
    let temp_dir = TempDir::new().expect("tempdir");
    let sprite_home = temp_dir.path().join("home");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
    fs::write(
        sprite_home.join(crate::CONFIG_TOML_FILE),
        r#"
model_provider = "ollama"
local_provider_default_model = "custom-oss"
"#,
    )
    .await
    .expect("write config");

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd)
        .loader_overrides(LoaderOverrides::without_host_requirements_for_tests())
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect("runtime config loads");

    assert_eq!(config.model_provider_id, "ollama");
    assert_eq!(config.model, "custom-oss");
}

#[tokio::test]
async fn runtime_config_applies_thread_user_and_session_config_sources() {
    let temp_dir = TempDir::new().expect("tempdir");
    let sprite_home = temp_dir.path().join("home");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
    let loader = StaticThreadConfigLoader::new(vec![
        ThreadConfigSource::User(UserThreadConfig {
            model: Some("thread-model".to_string()),
            ..Default::default()
        }),
        ThreadConfigSource::Session(SessionThreadConfig {
            model_provider: Some("thread-local".to_string()),
            model_providers: HashMap::from([(
                "thread-local".to_string(),
                ModelProviderInfo {
                    name: "thread-local".to_string(),
                    base_url: Some("http://127.0.0.1:9999/v1".to_string()),
                    wire_api: WireApi::Responses,
                    ..Default::default()
                },
            )]),
            features: BTreeMap::new(),
        }),
    ]);

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd)
        .load_options(ConfigLoadOptions {
            loader_overrides: LoaderOverrides::without_host_requirements_for_tests(),
            strict_config: true,
            ..Default::default()
        })
        .load_with(&TestFileSystem, &loader)
        .await
        .expect("runtime config loads");

    assert_eq!(config.model, "thread-model");
    assert_eq!(config.model_provider_id, "thread-local");
    assert_eq!(
        config.model_provider.base_url.as_deref(),
        Some("http://127.0.0.1:9999/v1")
    );
}

#[tokio::test]
async fn runtime_config_enforces_requirements_in_typed_view() {
    let temp_dir = TempDir::new().expect("tempdir");
    let sprite_home = temp_dir.path().join("home");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let system_requirements = temp_dir.path().join("requirements.toml");
    let cwd = AbsolutePathBuf::from_absolute_path(temp_dir.path()).expect("absolute cwd");
    fs::write(
        &system_requirements,
        r#"
allowed_approval_policies = ["on-request"]
allowed_approvals_reviewers = ["auto_review"]
allowed_web_search_modes = ["disabled"]

[mcp_servers.allowed.identity]
command = "allowed-mcp"

[hooks]
managed_dir = "/managed/hooks"

[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "python3 /managed/hooks/pre.py"
"#,
    )
    .await
    .expect("write requirements");
    fs::write(
        sprite_home.join(crate::CONFIG_TOML_FILE),
        r#"
approval_policy = "on-request"
approvals_reviewer = "user"
web_search = "disabled"

[mcp_servers.allowed]
command = "allowed-mcp"

[mcp_servers.blocked]
command = "blocked-mcp"

[hooks]
PreToolUse = [{ matcher = "shell", hooks = [{ type = "command", command = "echo user" }] }]
"#,
    )
    .await
    .expect("write config");

    let err = RuntimeConfig::builder()
        .sprite_home(sprite_home.clone())
        .cwd(cwd.clone())
        .loader_overrides(LoaderOverrides {
            system_requirements_path: Some(system_requirements.clone()),
            ..LoaderOverrides::without_host_requirements_for_tests()
        })
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect_err("disallowed approvals reviewer should fail");

    assert!(
        err.to_string().contains("approvals_reviewer"),
        "unexpected error: {err}"
    );

    fs::write(
        sprite_home.join(crate::CONFIG_TOML_FILE),
        r#"
approval_policy = "on-request"
approvals_reviewer = "auto_review"
web_search = "disabled"

[mcp_servers.allowed]
command = "allowed-mcp"

[mcp_servers.blocked]
command = "blocked-mcp"

[hooks]
PreToolUse = [{ matcher = "shell", hooks = [{ type = "command", command = "echo user" }] }]
"#,
    )
    .await
    .expect("rewrite config");

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd)
        .loader_overrides(LoaderOverrides {
            system_requirements_path: Some(system_requirements.clone()),
            ..LoaderOverrides::without_host_requirements_for_tests()
        })
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect("runtime config loads");

    assert_eq!(config.approval_policy, AskForApproval::OnRequest);
    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::AutoReview);
    assert_eq!(config.web_search, WebSearchMode::Disabled);
    assert!(config.mcp_servers["allowed"].enabled);
    assert!(!config.mcp_servers["blocked"].enabled);
    assert_eq!(
        config.mcp_servers["blocked"].disabled_reason,
        Some(McpServerDisabledReason::Requirements {
            source: RequirementSource::SystemRequirementsToml {
                file: AbsolutePathBuf::from_absolute_path(&system_requirements)
                    .expect("absolute requirements")
            }
        })
    );
    assert_eq!(config.hooks.events.handler_count(), 1);
    let first_hook_command = config
        .hooks
        .events
        .pre_tool_use
        .first()
        .and_then(|group| group.hooks.first())
        .expect("managed hook");
    assert!(format!("{first_hook_command:?}").contains("/managed/hooks/pre.py"));
}

#[tokio::test]
async fn runtime_config_uses_repo_root_for_project_trust_defaults() {
    let temp_dir = TempDir::new().expect("tempdir");
    let repo = temp_dir.path().join("repo");
    let nested = repo.join("nested").join("project");
    let sprite_home = temp_dir.path().join("home");
    fs::create_dir_all(repo.join(".git"))
        .await
        .expect(".git dir");
    fs::create_dir_all(&nested).await.expect("nested dir");
    fs::create_dir_all(&sprite_home).await.expect("home dir");
    let cwd = AbsolutePathBuf::from_absolute_path(&nested).expect("absolute cwd");
    fs::write(
        sprite_home.join(crate::CONFIG_TOML_FILE),
        format!(
            r#"
[windows]
sandbox = "unelevated"

[projects.{repo:?}]
trust_level = "trusted"
"#,
        ),
    )
    .await
    .expect("write config");

    let config = RuntimeConfig::builder()
        .sprite_home(sprite_home)
        .cwd(cwd)
        .loader_overrides(LoaderOverrides::without_host_requirements_for_tests())
        .load_with(&TestFileSystem, &crate::NoopThreadConfigLoader)
        .await
        .expect("runtime config loads");

    assert_eq!(
        config.active_permission_profile.id,
        BUILT_IN_PERMISSION_PROFILE_WORKSPACE
    );
}
