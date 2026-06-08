use super::*;
use async_trait::async_trait;
use file_system::CopyOptions;
use file_system::CreateDirectoryOptions;
use file_system::FileMetadata;
use file_system::FileSystemResult;
use file_system::FileSystemSandboxContext;
use file_system::ReadDirectoryEntry;
use file_system::RemoveOptions;
use pretty_assertions::assert_eq;
use std::path::Path;
use tempfile::tempdir;

struct TestFileSystem;

#[async_trait]
impl ExecutorFileSystem for TestFileSystem {
    async fn canonicalize(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<AbsolutePathBuf> {
        path.canonicalize()
    }

    async fn join(
        &self,
        base_path: &AbsolutePathBuf,
        path: &Path,
    ) -> FileSystemResult<AbsolutePathBuf> {
        Ok(base_path.join(path))
    }

    async fn parent(&self, path: &AbsolutePathBuf) -> FileSystemResult<Option<AbsolutePathBuf>> {
        Ok(path.parent())
    }

    async fn read_file(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<Vec<u8>> {
        tokio::fs::read(path.as_path()).await
    }

    async fn write_file(
        &self,
        _path: &AbsolutePathBuf,
        _contents: Vec<u8>,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        unimplemented!("test filesystem only supports reads")
    }

    async fn create_directory(
        &self,
        _path: &AbsolutePathBuf,
        _create_directory_options: CreateDirectoryOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        unimplemented!("test filesystem only supports reads")
    }

    async fn get_metadata(
        &self,
        _path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<FileMetadata> {
        unimplemented!("test filesystem only supports reads")
    }

    async fn read_directory(
        &self,
        _path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<Vec<ReadDirectoryEntry>> {
        unimplemented!("test filesystem only supports reads")
    }

    async fn remove(
        &self,
        _path: &AbsolutePathBuf,
        _remove_options: RemoveOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        unimplemented!("test filesystem only supports reads")
    }

    async fn copy(
        &self,
        _source_path: &AbsolutePathBuf,
        _destination_path: &AbsolutePathBuf,
        _copy_options: CopyOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        unimplemented!("test filesystem only supports reads")
    }
}

#[tokio::test]
async fn profile_v2_rejects_matching_legacy_profile_in_base_user_config() {
    let tmp = tempdir().expect("tempdir");
    let selected_config = tmp.path().join("work.config.toml");

    std::fs::write(
        tmp.path().join(CONFIG_TOML_FILE),
        r#"
model = "gpt-main"

[profiles.work]
model = "gpt-work"
"#,
    )
    .expect("write default user config");
    std::fs::write(&selected_config, r#"model = "gpt-work-v2""#)
        .expect("write selected user config");

    let mut overrides = LoaderOverrides::without_host_requirements_for_tests();
    overrides.user_config_path = Some(AbsolutePathBuf::resolve_path_against_base(
        "work.config.toml",
        tmp.path(),
    ));
    overrides.user_config_profile = Some("work".parse().expect("profile-v2 name"));

    let err = load_config_layers_state(
        &TestFileSystem,
        tmp.path(),
        /*cwd*/ None,
        &[],
        overrides,
        &crate::NoopThreadConfigLoader,
    )
    .await
    .expect_err("profile-v2 should reject a matching legacy profile in base user config");

    assert_eq!(
        err.kind(),
        io::ErrorKind::InvalidData,
        "a matching legacy profile should be a hard config error"
    );
    let message = err.to_string();
    assert!(
        message.contains("--profile `work` cannot be used"),
        "unexpected error message: {message}"
    );
    assert!(
        message.contains("config.toml"),
        "unexpected error message: {message}"
    );
    assert!(
        message.contains("[profiles.work]"),
        "unexpected error message: {message}"
    );
}

#[tokio::test]
async fn profile_v2_rejects_matching_legacy_profile_selector_in_base_user_config() {
    let tmp = tempdir().expect("tempdir");
    let selected_config = tmp.path().join("work.config.toml");

    std::fs::write(
        tmp.path().join(CONFIG_TOML_FILE),
        r#"
profile = "work"
model = "gpt-main"
"#,
    )
    .expect("write default user config");
    std::fs::write(&selected_config, r#"model = "gpt-work-v2""#)
        .expect("write selected user config");

    let mut overrides = LoaderOverrides::without_host_requirements_for_tests();
    overrides.user_config_path = Some(AbsolutePathBuf::resolve_path_against_base(
        "work.config.toml",
        tmp.path(),
    ));
    overrides.user_config_profile = Some("work".parse().expect("profile-v2 name"));

    let err = load_config_layers_state(
        &TestFileSystem,
        tmp.path(),
        /*cwd*/ None,
        &[],
        overrides,
        &crate::NoopThreadConfigLoader,
    )
    .await
    .expect_err("profile-v2 should reject a matching legacy profile selector");

    assert_eq!(
        err.kind(),
        io::ErrorKind::InvalidData,
        "a matching legacy profile selector should be a hard config error"
    );
    let message = err.to_string();
    assert!(
        message.contains("--profile `work` cannot be used"),
        "unexpected error message: {message}"
    );
    assert!(
        message.contains("profile = \"work\""),
        "unexpected error message: {message}"
    );
    assert!(
        message.contains("work.config.toml"),
        "unexpected error message: {message}"
    );
}

#[tokio::test]
async fn profile_v2_allows_unrelated_legacy_profiles_in_base_user_config() {
    let tmp = tempdir().expect("tempdir");
    let selected_config = tmp.path().join("work.config.toml");

    std::fs::write(
        tmp.path().join(CONFIG_TOML_FILE),
        r#"
model = "gpt-main"

[profiles.dev]
model = "gpt-dev"
"#,
    )
    .expect("write default user config");
    std::fs::write(&selected_config, r#"model = "gpt-work-v2""#)
        .expect("write selected user config");

    let mut overrides = LoaderOverrides::without_host_requirements_for_tests();
    overrides.user_config_path = Some(AbsolutePathBuf::resolve_path_against_base(
        "work.config.toml",
        tmp.path(),
    ));
    overrides.user_config_profile = Some("work".parse().expect("profile-v2 name"));

    load_config_layers_state(
        &TestFileSystem,
        tmp.path(),
        /*cwd*/ None,
        &[],
        overrides,
        &crate::NoopThreadConfigLoader,
    )
    .await
    .expect("profile-v2 should allow unrelated legacy profiles in base user config");
}

#[tokio::test]
async fn model_catalog_json_loads_from_effective_config() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(CONFIG_TOML_FILE),
        r#"model_catalog_json = "./models.json""#,
    )
    .expect("write user config");
    std::fs::write(
        tmp.path().join("models.json"),
        model_catalog_json("sprite-test"),
    )
    .expect("write catalog");

    let stack = load_config_layers_state(
        &TestFileSystem,
        tmp.path(),
        /*cwd*/ None,
        &[],
        LoaderOverrides::without_host_requirements_for_tests(),
        &crate::NoopThreadConfigLoader,
    )
    .await
    .expect("load config");

    let catalog = stack
        .load_model_catalog_json()
        .expect("load model catalog")
        .expect("catalog configured");

    assert_eq!(catalog.models.len(), 1);
    assert_eq!(catalog.models[0].slug, "sprite-test");
}

#[tokio::test]
async fn model_catalog_json_rejects_empty_catalog() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(CONFIG_TOML_FILE),
        r#"model_catalog_json = "./models.json""#,
    )
    .expect("write user config");
    std::fs::write(tmp.path().join("models.json"), r#"{"models":[]}"#).expect("write catalog");

    let stack = load_config_layers_state(
        &TestFileSystem,
        tmp.path(),
        /*cwd*/ None,
        &[],
        LoaderOverrides::without_host_requirements_for_tests(),
        &crate::NoopThreadConfigLoader,
    )
    .await
    .expect("load config");

    let err = stack
        .load_model_catalog_json()
        .expect_err("empty catalog should fail");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("must contain at least one model"));
}

#[tokio::test]
async fn model_catalog_json_rejects_invalid_json() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(CONFIG_TOML_FILE),
        r#"model_catalog_json = "./models.json""#,
    )
    .expect("write user config");
    std::fs::write(tmp.path().join("models.json"), "not json").expect("write catalog");

    let stack = load_config_layers_state(
        &TestFileSystem,
        tmp.path(),
        /*cwd*/ None,
        &[],
        LoaderOverrides::without_host_requirements_for_tests(),
        &crate::NoopThreadConfigLoader,
    )
    .await
    .expect("load config");

    let err = stack
        .load_model_catalog_json()
        .expect_err("invalid JSON should fail");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(
        err.to_string()
            .contains("failed to parse model_catalog_json")
    );
}

fn model_catalog_json(slug: &str) -> String {
    format!(
        r#"{{
  "models": [
    {{
      "slug": "{slug}",
      "display_name": "Sprite Test",
      "description": null,
      "supported_reasoning_levels": [],
      "shell_type": "shell_command",
      "visibility": "list",
      "supported_in_api": true,
      "priority": 1,
      "upgrade": null,
      "base_instructions": "base",
      "model_messages": null,
      "supports_reasoning_summaries": false,
      "default_reasoning_summary": "auto",
      "support_verbosity": false,
      "default_verbosity": null,
      "apply_patch_tool_type": null,
      "truncation_policy": {{
        "mode": "bytes",
        "limit": 10000
      }},
      "supports_parallel_tool_calls": false,
      "supports_image_detail_original": false,
      "context_window": null,
      "auto_compact_token_limit": null,
      "effective_context_window_percent": 95,
      "experimental_supported_tools": [],
      "input_modalities": ["text"]
    }}
  ]
}}"#
    )
}
