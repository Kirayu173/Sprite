use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use app_protocol::Config;
use app_protocol::ConfigBatchWriteParams;
use app_protocol::ConfigEdit as ProtocolConfigEdit;
use app_protocol::ConfigLayerMetadata;
use app_protocol::ConfigLayerSource;
use app_protocol::ConfigReadParams;
use app_protocol::ConfigReadResponse;
use app_protocol::ConfigValueWriteParams;
use app_protocol::ConfigWriteErrorCode;
use app_protocol::ConfigWriteResponse;
use app_protocol::WriteStatus;
use file_system::ExecutorFileSystem;
use git_utils::resolve_root_git_project_for_trust;
use thiserror::Error;
use toml::Value as TomlValue;
use utils_absolute_path::AbsolutePathBuf;
use utils_path::resolve_symlink_write_paths;
use utils_path::write_atomically;

use crate::CONFIG_TOML_FILE;
use crate::ConfigLayerStack;
use crate::ConfigLayerStackOrdering;
use crate::ConfigLoadOptions;
use crate::LoaderOverrides;
use crate::RuntimeConfig;
use crate::config_toml::ConfigToml;
use crate::config_error_from_ignored_toml_fields;
use crate::loader::load_config_layers_state;
use crate::loader::resolve_relative_paths_in_config_toml;
use crate::manager_edit::apply_protocol_edit;
use crate::manager_edit::parse_key_path;
use crate::manager_edit::read_or_create_document;
use crate::manager_overrides::first_overridden_metadata;
use crate::thread_config::NoopThreadConfigLoader;
use crate::thread_config::ThreadConfigLoader;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ConfigManagerError {
    code: ConfigWriteErrorCode,
    message: String,
}

impl ConfigManagerError {
    pub fn new(code: ConfigWriteErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ConfigWriteErrorCode {
        self.code.clone()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone)]
pub struct ConfigManager {
    sprite_home: PathBuf,
    cwd: AbsolutePathBuf,
    load_options: ConfigLoadOptions,
    model_provider_override: Option<String>,
    thread_config_loader: Arc<dyn ThreadConfigLoader>,
}

impl std::fmt::Debug for ConfigManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigManager")
            .field("sprite_home", &self.sprite_home)
            .field("cwd", &self.cwd)
            .field("load_options", &self.load_options)
            .field("model_provider_override", &self.model_provider_override)
            .finish_non_exhaustive()
    }
}

impl ConfigManager {
    pub fn builder() -> ConfigManagerBuilder {
        ConfigManagerBuilder::default()
    }

    pub fn new(sprite_home: impl Into<PathBuf>, cwd: AbsolutePathBuf) -> Self {
        Self::builder().sprite_home(sprite_home).cwd(cwd).build()
    }

    pub async fn read(
        &self,
        params: ConfigReadParams,
    ) -> Result<ConfigReadResponse, ConfigManagerError> {
        let cwd = self.resolve_read_cwd(params.cwd.as_deref())?;
        let stack = self
            .load_stack(Some(cwd))
            .await
            .map_err(manager_error_from_io)?;
        let effective_config = stack.effective_config();
        let config = protocol_config_from_toml(effective_config)?;
        let layers = if params.include_layers {
            Some(
                stack
                    .get_layers(
                        ConfigLayerStackOrdering::LowestPrecedenceFirst,
                        /*include_disabled*/ true,
                    )
                    .into_iter()
                    .map(|layer| layer.as_layer())
                    .collect(),
            )
        } else {
            None
        };

        Ok(ConfigReadResponse {
            config,
            origins: stack.origins(),
            layers,
        })
    }

    pub async fn write_value(
        &self,
        params: ConfigValueWriteParams,
    ) -> Result<ConfigWriteResponse, ConfigManagerError> {
        self.write(ConfigWriteRequest {
            edits: vec![ProtocolConfigEdit {
                key_path: params.key_path,
                value: params.value,
                merge_strategy: params.merge_strategy,
            }],
            file_path: params.file_path,
            expected_version: params.expected_version,
        })
        .await
    }

    pub async fn batch_write(
        &self,
        params: ConfigBatchWriteParams,
    ) -> Result<ConfigWriteResponse, ConfigManagerError> {
        self.write(ConfigWriteRequest {
            edits: params.edits,
            file_path: params.file_path,
            expected_version: params.expected_version,
        })
        .await
    }

    async fn write(
        &self,
        request: ConfigWriteRequest,
    ) -> Result<ConfigWriteResponse, ConfigManagerError> {
        let stack = self
            .load_stack(Some(self.cwd.clone()))
            .await
            .map_err(manager_error_from_io)?;
        let target = self.write_target(&stack, request.file_path.as_deref())?;
        if let Some(expected_version) = request.expected_version.as_deref()
            && expected_version != target.version
        {
            return Err(ConfigManagerError::new(
                ConfigWriteErrorCode::ConfigVersionConflict,
                format!(
                    "config version conflict for `{}`: expected `{expected_version}`, found `{}`",
                    target.file_path.as_path().display(),
                    target.version
                ),
            ));
        }

        let write_paths = resolve_symlink_write_paths(target.file_path.as_path())
            .map_err(manager_error_from_io)?;
        let mut doc = read_or_create_document(write_paths.read_path.as_deref())?;
        let mut edited_paths = Vec::new();
        for edit in request.edits {
            let segments = parse_key_path(&edit.key_path)?;
            apply_protocol_edit(&mut doc, &segments, edit.value, edit.merge_strategy)?;
            edited_paths.push(segments);
        }

        let contents = doc.to_string();
        validate_config_document(&target.file_path, &contents)?;
        let parsed = parse_config_document(&target.file_path, &contents)?;
        let base_dir = target.file_path.parent().ok_or_else(|| {
            ConfigManagerError::new(
                ConfigWriteErrorCode::ConfigPathNotFound,
                format!(
                    "config file `{}` has no parent directory",
                    target.file_path.as_path().display()
                ),
            )
        })?;
        let resolved = resolve_relative_paths_in_config_toml(parsed, base_dir.as_path())
            .map_err(manager_error_from_io)?;
        let updated_stack = stack.with_user_config(&target.file_path, resolved);
        let fs = NativeConfigFileSystem;
        let repo_root = resolve_root_git_project_for_trust(&fs, &self.cwd).await;
        RuntimeConfig::from_layer_stack(
            updated_stack.clone(),
            &self.sprite_home,
            self.cwd.clone(),
            repo_root,
            self.model_provider_override.as_deref(),
        )
        .map_err(manager_error_from_io)?;

        if let Some(parent) = write_paths.write_path.parent() {
            std::fs::create_dir_all(parent).map_err(manager_error_from_io)?;
        }
        write_atomically(&write_paths.write_path, &contents).map_err(manager_error_from_io)?;

        let target_metadata = target_metadata_for_path(&updated_stack, &target.file_path)?;
        let overridden_metadata =
            first_overridden_metadata(&updated_stack, &target_metadata, &edited_paths);
        let status = if overridden_metadata.is_some() {
            WriteStatus::OkOverridden
        } else {
            WriteStatus::Ok
        };

        Ok(ConfigWriteResponse {
            status,
            version: target_metadata.version,
            file_path: target.file_path,
            overridden_metadata,
        })
    }

    async fn load_stack(&self, cwd: Option<AbsolutePathBuf>) -> io::Result<ConfigLayerStack> {
        let fs = NativeConfigFileSystem;
        load_config_layers_state(
            &fs,
            &self.sprite_home,
            cwd,
            &[],
            self.load_options.clone(),
            self.thread_config_loader.as_ref(),
        )
        .await
    }

    fn write_target(
        &self,
        stack: &ConfigLayerStack,
        requested_path: Option<&str>,
    ) -> Result<WriteTarget, ConfigManagerError> {
        match requested_path {
            None => {
                let layer = stack.get_active_user_layer().ok_or_else(|| {
                    ConfigManagerError::new(
                        ConfigWriteErrorCode::UserLayerNotFound,
                        "no writable user config layer is loaded",
                    )
                })?;
                let ConfigLayerSource::User { file, .. } = &layer.name else {
                    return Err(ConfigManagerError::new(
                        ConfigWriteErrorCode::UserLayerNotFound,
                        "active writable config layer is not a user layer",
                    ));
                };
                Ok(WriteTarget {
                    file_path: file.clone(),
                    version: layer.version.clone(),
                })
            }
            Some(path) => {
                let requested = self.resolve_write_path(path)?;
                let layer = stack
                    .get_layers(
                        ConfigLayerStackOrdering::LowestPrecedenceFirst,
                        /*include_disabled*/ true,
                    )
                    .into_iter()
                    .find(|layer| layer_file_path(layer).as_ref() == Some(&requested));
                let Some(layer) = layer else {
                    return Err(ConfigManagerError::new(
                        ConfigWriteErrorCode::ConfigPathNotFound,
                        format!(
                            "config path `{}` is not part of the loaded config stack",
                            requested.as_path().display()
                        ),
                    ));
                };
                if !matches!(layer.name, ConfigLayerSource::User { .. }) {
                    return Err(ConfigManagerError::new(
                        ConfigWriteErrorCode::ConfigLayerReadonly,
                        format!(
                            "config path `{}` is not a writable user config layer",
                            requested.as_path().display()
                        ),
                    ));
                }
                Ok(WriteTarget {
                    file_path: requested,
                    version: layer.version.clone(),
                })
            }
        }
    }

    fn resolve_read_cwd(&self, cwd: Option<&str>) -> Result<AbsolutePathBuf, ConfigManagerError> {
        match cwd {
            Some(cwd) => self.resolve_path(cwd),
            None => Ok(self.cwd.clone()),
        }
    }

    fn resolve_write_path(&self, path: &str) -> Result<AbsolutePathBuf, ConfigManagerError> {
        self.resolve_path(path)
    }

    fn resolve_path(&self, path: &str) -> Result<AbsolutePathBuf, ConfigManagerError> {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            AbsolutePathBuf::from_absolute_path(path).map_err(manager_error_from_io)
        } else {
            Ok(self.cwd.join(path))
        }
    }
}

#[derive(Clone)]
pub struct ConfigManagerBuilder {
    sprite_home: Option<PathBuf>,
    cwd: Option<AbsolutePathBuf>,
    load_options: ConfigLoadOptions,
    model_provider_override: Option<String>,
    thread_config_loader: Arc<dyn ThreadConfigLoader>,
}

impl std::fmt::Debug for ConfigManagerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigManagerBuilder")
            .field("sprite_home", &self.sprite_home)
            .field("cwd", &self.cwd)
            .field("load_options", &self.load_options)
            .field("model_provider_override", &self.model_provider_override)
            .finish_non_exhaustive()
    }
}

impl Default for ConfigManagerBuilder {
    fn default() -> Self {
        Self {
            sprite_home: None,
            cwd: None,
            load_options: ConfigLoadOptions::default(),
            model_provider_override: None,
            thread_config_loader: Arc::new(NoopThreadConfigLoader),
        }
    }
}

impl ConfigManagerBuilder {
    pub fn sprite_home(mut self, sprite_home: impl Into<PathBuf>) -> Self {
        self.sprite_home = Some(sprite_home.into());
        self
    }

    pub fn cwd(mut self, cwd: AbsolutePathBuf) -> Self {
        self.cwd = Some(cwd);
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

    pub fn thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.load_options.thread_id = Some(thread_id.into());
        self
    }

    pub fn thread_config_loader(
        mut self,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) -> Self {
        self.thread_config_loader = thread_config_loader;
        self
    }

    pub fn build(self) -> ConfigManager {
        let sprite_home = self.sprite_home.unwrap_or_else(default_sprite_home);
        let cwd = self
            .cwd
            .unwrap_or_else(|| AbsolutePathBuf::current_dir().expect("current dir is absolute"));
        ConfigManager {
            sprite_home,
            cwd,
            load_options: self.load_options,
            model_provider_override: self.model_provider_override,
            thread_config_loader: self.thread_config_loader,
        }
    }
}

#[derive(Debug)]
struct ConfigWriteRequest {
    edits: Vec<ProtocolConfigEdit>,
    file_path: Option<String>,
    expected_version: Option<String>,
}

#[derive(Debug)]
struct WriteTarget {
    file_path: AbsolutePathBuf,
    version: String,
}

#[derive(Debug, Default)]
struct NativeConfigFileSystem;

impl ExecutorFileSystem for NativeConfigFileSystem {}

fn default_sprite_home() -> PathBuf {
    if let Some(value) = std::env::var_os("SPRITE_HOME") {
        return PathBuf::from(value);
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".sprite"))
        .unwrap_or_else(|| PathBuf::from(".sprite"))
}

fn protocol_config_from_toml(config: TomlValue) -> Result<Config, ConfigManagerError> {
    let json = serde_json::to_value(config).map_err(manager_error_from_serde)?;
    serde_json::from_value(json).map_err(manager_error_from_serde)
}

fn validate_config_document(
    file_path: &AbsolutePathBuf,
    contents: &str,
) -> Result<(), ConfigManagerError> {
    if let Some(error) = config_error_from_ignored_toml_fields::<ConfigToml>(file_path, contents) {
        return Err(ConfigManagerError::new(
            ConfigWriteErrorCode::ConfigValidationError,
            error.message,
        ));
    }
    Ok(())
}

fn parse_config_document(
    file_path: &AbsolutePathBuf,
    contents: &str,
) -> Result<TomlValue, ConfigManagerError> {
    toml::from_str(contents).map_err(|err| {
        ConfigManagerError::new(
            ConfigWriteErrorCode::ConfigValidationError,
            format!(
                "failed to parse config file `{}`: {err}",
                file_path.as_path().display()
            ),
        )
    })
}

fn target_metadata_for_path(
    stack: &ConfigLayerStack,
    file_path: &AbsolutePathBuf,
) -> Result<ConfigLayerMetadata, ConfigManagerError> {
    stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        )
        .into_iter()
        .find_map(|layer| match &layer.name {
            ConfigLayerSource::User { file, .. } if file == file_path => Some(layer.metadata()),
            _ => None,
        })
        .ok_or_else(|| {
            ConfigManagerError::new(
                ConfigWriteErrorCode::UserLayerNotFound,
                "written user config layer was not present after validation",
            )
        })
}

fn layer_file_path(layer: &crate::ConfigLayerEntry) -> Option<AbsolutePathBuf> {
    match &layer.name {
        ConfigLayerSource::System { file } => Some(file.clone()),
        ConfigLayerSource::User { file, .. } => Some(file.clone()),
        ConfigLayerSource::Project { project_config_dir } => {
            Some(project_config_dir.join(CONFIG_TOML_FILE))
        }
        ConfigLayerSource::SessionFlags => None,
    }
}

fn manager_error_from_io(err: io::Error) -> ConfigManagerError {
    ConfigManagerError::new(ConfigWriteErrorCode::ConfigValidationError, err.to_string())
}

fn manager_error_from_serde(err: serde_json::Error) -> ConfigManagerError {
    ConfigManagerError::new(ConfigWriteErrorCode::ConfigValidationError, err.to_string())
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
