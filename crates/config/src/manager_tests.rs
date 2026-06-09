use std::fs;
use std::sync::Arc;

use app_protocol::ConfigBatchWriteParams;
use app_protocol::ConfigEdit;
use app_protocol::ConfigLayerSource;
use app_protocol::ConfigReadParams;
use app_protocol::ConfigValueWriteParams;
use app_protocol::ConfigWriteErrorCode;
use app_protocol::MergeStrategy;
use app_protocol::WriteStatus;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::DynamicThreadConfigLoader;
use crate::SessionThreadConfig;
use crate::ThreadConfigSource;

fn test_manager(temp_dir: &TempDir) -> ConfigManager {
    let sprite_home = temp_dir.path().join("home");
    let cwd = temp_dir.path().join("repo");
    fs::create_dir_all(&sprite_home).expect("sprite home");
    fs::create_dir_all(&cwd).expect("cwd");

    ConfigManager::new(
        sprite_home,
        AbsolutePathBuf::from_absolute_path(cwd).expect("absolute cwd"),
    )
}

fn config_file(temp_dir: &TempDir) -> std::path::PathBuf {
    temp_dir.path().join("home").join(CONFIG_TOML_FILE)
}

fn trusted_project_toml(temp_dir: &TempDir) -> String {
    let project = temp_dir
        .path()
        .join("repo")
        .display()
        .to_string()
        .replace('\\', "\\\\");
    format!(
        r#"
[projects."{project}"]
trust_level = "trusted"
"#
    )
}

#[tokio::test]
async fn read_returns_effective_config_and_layers() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(temp_dir.path().join("home")).expect("home");
    fs::create_dir_all(temp_dir.path().join("repo")).expect("repo");
    fs::write(config_file(&temp_dir), r#"model = "user-model""#).expect("config");
    let manager = test_manager(&temp_dir);

    let response = manager
        .read(ConfigReadParams {
            include_layers: true,
            cwd: None,
        })
        .await
        .expect("read config");

    assert_eq!(response.config.model.as_deref(), Some("user-model"));
    assert!(response.origins.contains_key("model"));
    assert!(response.layers.expect("layers").iter().any(|layer| {
        matches!(layer.name, ConfigLayerSource::User { .. })
            && layer.config.get("model").and_then(serde_json::Value::as_str) == Some("user-model")
    }));
}

#[tokio::test]
async fn value_write_updates_user_config_and_reports_new_version() {
    let temp_dir = TempDir::new().expect("tempdir");
    let manager = test_manager(&temp_dir);

    let response = manager
        .write_value(ConfigValueWriteParams {
            key_path: "model".to_string(),
            value: json!("gpt-local"),
            merge_strategy: MergeStrategy::Replace,
            file_path: None,
            expected_version: None,
        })
        .await
        .expect("write config");

    assert_eq!(response.status, WriteStatus::Ok);
    assert_eq!(response.file_path.as_path(), config_file(&temp_dir).as_path());
    assert!(response.version.starts_with("sha256:"));
    assert_eq!(
        fs::read_to_string(config_file(&temp_dir)).expect("read written config"),
        "model = \"gpt-local\"\n"
    );
}

#[tokio::test]
async fn batch_write_upserts_nested_tables() {
    let temp_dir = TempDir::new().expect("tempdir");
    let manager = test_manager(&temp_dir);

    manager
        .batch_write(ConfigBatchWriteParams {
            edits: vec![ConfigEdit {
                key_path: "desktop".to_string(),
                value: json!({ "theme": "dark" }),
                merge_strategy: MergeStrategy::Upsert,
            }],
            file_path: None,
            expected_version: None,
            reload_user_config: false,
        })
        .await
        .expect("first write");
    manager
        .batch_write(ConfigBatchWriteParams {
            edits: vec![ConfigEdit {
                key_path: "desktop".to_string(),
                value: json!({ "density": "compact" }),
                merge_strategy: MergeStrategy::Upsert,
            }],
            file_path: None,
            expected_version: None,
            reload_user_config: false,
        })
        .await
        .expect("second write");

    let written = fs::read_to_string(config_file(&temp_dir)).expect("read config");
    assert!(written.contains("theme = \"dark\""));
    assert!(written.contains("density = \"compact\""));
}

#[tokio::test]
async fn expected_version_conflict_is_rejected() {
    let temp_dir = TempDir::new().expect("tempdir");
    let manager = test_manager(&temp_dir);

    let err = manager
        .write_value(ConfigValueWriteParams {
            key_path: "model".to_string(),
            value: json!("gpt-local"),
            merge_strategy: MergeStrategy::Replace,
            file_path: None,
            expected_version: Some("sha256:stale".to_string()),
        })
        .await
        .expect_err("version conflict");

    assert_eq!(err.code(), ConfigWriteErrorCode::ConfigVersionConflict);
    assert!(!config_file(&temp_dir).exists());
}

#[tokio::test]
async fn unknown_config_field_is_rejected_before_write() {
    let temp_dir = TempDir::new().expect("tempdir");
    let manager = test_manager(&temp_dir);

    let err = manager
        .write_value(ConfigValueWriteParams {
            key_path: "not_a_real_config_key".to_string(),
            value: json!(true),
            merge_strategy: MergeStrategy::Replace,
            file_path: None,
            expected_version: None,
        })
        .await
        .expect_err("unknown key");

    assert_eq!(err.code(), ConfigWriteErrorCode::ConfigValidationError);
    assert!(err.message().contains("unknown configuration field"));
    assert!(!config_file(&temp_dir).exists());
}

#[tokio::test]
async fn higher_precedence_project_layer_reports_overridden_write() {
    let temp_dir = TempDir::new().expect("tempdir");
    let manager = test_manager(&temp_dir);
    fs::create_dir_all(temp_dir.path().join("repo").join(".sprite")).expect("project config dir");
    fs::write(
        temp_dir.path().join("repo").join(".sprite").join(CONFIG_TOML_FILE),
        r#"model = "project-model""#,
    )
    .expect("project config");
    fs::write(
        config_file(&temp_dir),
        trusted_project_toml(&temp_dir),
    )
    .expect("user trust");

    let response = manager
        .write_value(ConfigValueWriteParams {
            key_path: "model".to_string(),
            value: json!("user-model"),
            merge_strategy: MergeStrategy::Replace,
            file_path: None,
            expected_version: None,
        })
        .await
        .expect("write config");

    assert_eq!(response.status, WriteStatus::OkOverridden);
    let overridden = response.overridden_metadata.expect("overridden metadata");
    assert!(matches!(
        overridden.overriding_layer.name,
        ConfigLayerSource::Project { .. }
    ));
    assert_eq!(overridden.effective_value, json!("project-model"));
}

#[tokio::test]
async fn explicit_project_config_path_is_readonly() {
    let temp_dir = TempDir::new().expect("tempdir");
    let manager = test_manager(&temp_dir);
    let project_dir = temp_dir.path().join("repo").join(".sprite");
    fs::create_dir_all(&project_dir).expect("project config dir");
    fs::write(project_dir.join(CONFIG_TOML_FILE), r#"model = "project-model""#)
        .expect("project config");
    fs::write(
        config_file(&temp_dir),
        trusted_project_toml(&temp_dir),
    )
    .expect("user trust");

    let err = manager
        .write_value(ConfigValueWriteParams {
            key_path: "model".to_string(),
            value: json!("user-model"),
            merge_strategy: MergeStrategy::Replace,
            file_path: Some(project_dir.join(CONFIG_TOML_FILE).display().to_string()),
            expected_version: None,
        })
        .await
        .expect_err("readonly project config");

    assert_eq!(err.code(), ConfigWriteErrorCode::ConfigLayerReadonly);
}

#[tokio::test]
async fn dynamic_thread_loader_uses_thread_specific_sources() {
    let temp_dir = TempDir::new().expect("tempdir");
    let loader = DynamicThreadConfigLoader::new();
    loader.set_default_sources(vec![ThreadConfigSource::Session(SessionThreadConfig {
        model_provider: Some("default-provider".to_string()),
        ..Default::default()
    })]);
    loader.set_thread_sources(
        "thread-1",
        vec![ThreadConfigSource::Session(SessionThreadConfig {
            model_provider: Some("thread-provider".to_string()),
            ..Default::default()
        })],
    );
    let manager = ConfigManager::builder()
        .sprite_home(temp_dir.path().join("home"))
        .cwd(AbsolutePathBuf::from_absolute_path(temp_dir.path().join("repo")).expect("cwd"))
        .thread_id("thread-1")
        .thread_config_loader(Arc::new(loader))
        .build();
    fs::create_dir_all(temp_dir.path().join("home")).expect("home");
    fs::create_dir_all(temp_dir.path().join("repo")).expect("repo");

    let response = manager
        .read(ConfigReadParams {
            include_layers: false,
            cwd: None,
        })
        .await
        .expect("read config");

    assert_eq!(
        response.config.model_provider.as_deref(),
        Some("thread-provider")
    );
}
