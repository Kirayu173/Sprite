use std::collections::BTreeMap;
use std::collections::HashMap;

use file_system::ExecutorFileSystem;
use model_provider_info::ModelProviderInfo;
use model_provider_info::WireApi;
use pretty_assertions::assert_eq;
use runtime_protocol::models::BUILT_IN_PERMISSION_PROFILE_READ_ONLY;
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
