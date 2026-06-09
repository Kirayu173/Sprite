use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use app_protocol::ConfigLayerSource;
use async_trait::async_trait;
use model_provider_info::ModelProviderInfo;
use thiserror::Error;
use toml::Value as TomlValue;
use utils_absolute_path::AbsolutePathBuf;

use crate::ConfigLayerEntry;

/// Context available to implementations when loading thread-scoped config.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThreadConfigContext {
    pub thread_id: Option<String>,
    pub cwd: Option<AbsolutePathBuf>,
}

/// Config values owned by the service that starts or manages the session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionThreadConfig {
    pub model_provider: Option<String>,
    pub model_providers: HashMap<String, ModelProviderInfo>,
    pub features: BTreeMap<String, bool>,
}

/// Config values owned by the authenticated user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserThreadConfig {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub model_providers: HashMap<String, ModelProviderInfo>,
    pub features: BTreeMap<String, bool>,
}

/// A typed config payload paired with the authority that produced it.
#[derive(Clone, Debug, PartialEq)]
pub enum ThreadConfigSource {
    Session(SessionThreadConfig),
    User(UserThreadConfig),
}

/// Stable category for failures returned while loading thread config.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadConfigLoadErrorCode {
    Auth,
    Timeout,
    Parse,
    RequestFailed,
    Internal,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct ThreadConfigLoadError {
    code: ThreadConfigLoadErrorCode,
    message: String,
    status_code: Option<u16>,
}

impl ThreadConfigLoadError {
    pub fn new(
        code: ThreadConfigLoadErrorCode,
        status_code: Option<u16>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            status_code,
        }
    }

    pub fn code(&self) -> ThreadConfigLoadErrorCode {
        self.code
    }

    pub fn status_code(&self) -> Option<u16> {
        self.status_code
    }
}

/// Loads typed config sources for a new thread.
///
/// Implementations should fetch only the source-specific config they own and
/// return typed payloads without applying precedence or merge rules. Callers
/// are responsible for resolving the returned sources into the effective
/// runtime config.
#[async_trait]
pub trait ThreadConfigLoader: Send + Sync {
    /// Load source-specific typed config.
    ///
    /// Implementations should keep this method focused on fetching and parsing
    /// their owned sources. Most callers should use [`Self::load_config_layers`]
    /// so precedence and merging continue through the ordinary config layer
    /// stack.
    async fn load(
        &self,
        context: ThreadConfigContext,
    ) -> Result<Vec<ThreadConfigSource>, ThreadConfigLoadError>;

    async fn load_config_layers(
        &self,
        context: ThreadConfigContext,
    ) -> Result<Vec<ConfigLayerEntry>, ThreadConfigLoadError> {
        let sources = self.load(context).await?;
        sources
            .into_iter()
            .map(thread_config_source_to_layer)
            .collect::<Result<Vec<_>, _>>()
            .map(|layers| layers.into_iter().flatten().collect())
    }
}

/// Loader backed by a static set of typed thread config sources.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StaticThreadConfigLoader {
    sources: Vec<ThreadConfigSource>,
}

impl StaticThreadConfigLoader {
    pub fn new(sources: Vec<ThreadConfigSource>) -> Self {
        Self { sources }
    }
}

#[async_trait]
impl ThreadConfigLoader for StaticThreadConfigLoader {
    async fn load(
        &self,
        _context: ThreadConfigContext,
    ) -> Result<Vec<ThreadConfigSource>, ThreadConfigLoadError> {
        Ok(self.sources.clone())
    }
}

/// Loader used when no external thread config source is configured.
#[derive(Clone, Debug, Default)]
pub struct NoopThreadConfigLoader;

#[async_trait]
impl ThreadConfigLoader for NoopThreadConfigLoader {
    async fn load(
        &self,
        _context: ThreadConfigContext,
    ) -> Result<Vec<ThreadConfigSource>, ThreadConfigLoadError> {
        Ok(Vec::new())
    }
}

/// Mutable Sprite-owned thread config source.
///
/// Later runtime/app-server phases can update this loader directly without
/// reintroducing the removed official remote config endpoint.
#[derive(Clone, Debug, Default)]
pub struct DynamicThreadConfigLoader {
    default_sources: Arc<RwLock<Vec<ThreadConfigSource>>>,
    thread_sources: Arc<RwLock<HashMap<String, Vec<ThreadConfigSource>>>>,
}

impl DynamicThreadConfigLoader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_default_sources(&self, sources: Vec<ThreadConfigSource>) {
        *self
            .default_sources
            .write()
            .expect("thread config lock poisoned") = sources;
    }

    pub fn set_thread_sources(
        &self,
        thread_id: impl Into<String>,
        sources: Vec<ThreadConfigSource>,
    ) {
        self.thread_sources
            .write()
            .expect("thread config lock poisoned")
            .insert(thread_id.into(), sources);
    }

    pub fn clear_thread_sources(&self, thread_id: &str) {
        self.thread_sources
            .write()
            .expect("thread config lock poisoned")
            .remove(thread_id);
    }
}

#[async_trait]
impl ThreadConfigLoader for DynamicThreadConfigLoader {
    async fn load(
        &self,
        context: ThreadConfigContext,
    ) -> Result<Vec<ThreadConfigSource>, ThreadConfigLoadError> {
        if let Some(thread_id) = context.thread_id
            && let Some(sources) = self
                .thread_sources
                .read()
                .expect("thread config lock poisoned")
                .get(&thread_id)
                .cloned()
        {
            return Ok(sources);
        }

        Ok(self
            .default_sources
            .read()
            .expect("thread config lock poisoned")
            .clone())
    }
}

fn thread_config_source_to_layer(
    source: ThreadConfigSource,
) -> Result<Option<ConfigLayerEntry>, ThreadConfigLoadError> {
    match source {
        ThreadConfigSource::Session(config) => {
            let config = session_thread_config_to_toml(config)?;
            if is_empty_table(&config) {
                Ok(None)
            } else {
                Ok(Some(ConfigLayerEntry::new(
                    ConfigLayerSource::SessionFlags,
                    config,
                )))
            }
        }
        ThreadConfigSource::User(config) => {
            let config = user_thread_config_to_toml(config)?;
            if is_empty_table(&config) {
                Ok(None)
            } else {
                Ok(Some(ConfigLayerEntry::new(
                    ConfigLayerSource::SessionFlags,
                    config,
                )))
            }
        }
    }
}

fn is_empty_table(config: &TomlValue) -> bool {
    config.as_table().is_some_and(toml::map::Map::is_empty)
}

fn session_thread_config_to_toml(
    config: SessionThreadConfig,
) -> Result<TomlValue, ThreadConfigLoadError> {
    let mut table = toml::map::Map::new();

    if let Some(model_provider) = config.model_provider {
        table.insert(
            "model_provider".to_string(),
            TomlValue::String(model_provider),
        );
    }

    if !config.model_providers.is_empty() {
        let model_providers = TomlValue::try_from(config.model_providers).map_err(|err| {
            ThreadConfigLoadError::new(
                ThreadConfigLoadErrorCode::Parse,
                /*status_code*/ None,
                format!("failed to convert session model providers to config TOML: {err}"),
            )
        })?;
        table.insert("model_providers".to_string(), model_providers);
    }

    if !config.features.is_empty() {
        let features = config
            .features
            .into_iter()
            .map(|(feature, enabled)| (feature, TomlValue::Boolean(enabled)))
            .collect();
        table.insert("features".to_string(), TomlValue::Table(features));
    }

    Ok(TomlValue::Table(table))
}

fn user_thread_config_to_toml(
    config: UserThreadConfig,
) -> Result<TomlValue, ThreadConfigLoadError> {
    let mut table = toml::map::Map::new();

    if let Some(model) = config.model {
        table.insert("model".to_string(), TomlValue::String(model));
    }

    if let Some(model_provider) = config.model_provider {
        table.insert(
            "model_provider".to_string(),
            TomlValue::String(model_provider),
        );
    }

    if !config.model_providers.is_empty() {
        let model_providers = TomlValue::try_from(config.model_providers).map_err(|err| {
            ThreadConfigLoadError::new(
                ThreadConfigLoadErrorCode::Parse,
                /*status_code*/ None,
                format!("failed to convert user model providers to config TOML: {err}"),
            )
        })?;
        table.insert("model_providers".to_string(), model_providers);
    }

    if !config.features.is_empty() {
        let features = config
            .features
            .into_iter()
            .map(|(feature, enabled)| (feature, TomlValue::Boolean(enabled)))
            .collect();
        table.insert("features".to_string(), TomlValue::Table(features));
    }

    Ok(TomlValue::Table(table))
}

#[cfg(test)]
mod tests {
    use model_provider_info::ModelProviderInfo;
    use model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    use super::*;

    #[tokio::test]
    async fn loader_returns_session_and_user_sources() {
        let loader = StaticThreadConfigLoader::new(vec![
            ThreadConfigSource::Session(SessionThreadConfig {
                model_provider: Some("local".to_string()),
                model_providers: HashMap::from([("local".to_string(), test_provider("local"))]),
                features: BTreeMap::from([("plugins".to_string(), false)]),
            }),
            ThreadConfigSource::User(UserThreadConfig::default()),
        ]);

        let sources = loader
            .load(ThreadConfigContext {
                thread_id: Some("thread-1".to_string()),
                ..Default::default()
            })
            .await
            .expect("thread config loads");

        assert_eq!(
            sources,
            vec![
                ThreadConfigSource::Session(SessionThreadConfig {
                    model_provider: Some("local".to_string()),
                    model_providers: HashMap::from([("local".to_string(), test_provider("local"))]),
                    features: BTreeMap::from([("plugins".to_string(), false)]),
                }),
                ThreadConfigSource::User(UserThreadConfig::default()),
            ]
        );
    }

    #[tokio::test]
    async fn loader_translates_sources_to_config_layers() {
        let loader = StaticThreadConfigLoader::new(vec![
            ThreadConfigSource::User(UserThreadConfig {
                model: Some("gpt-user".to_string()),
                ..Default::default()
            }),
            ThreadConfigSource::Session(SessionThreadConfig {
                model_provider: Some("local".to_string()),
                model_providers: HashMap::from([("local".to_string(), test_provider("local"))]),
                features: BTreeMap::from([("plugins".to_string(), false)]),
            }),
        ]);
        let layers = loader
            .load_config_layers(ThreadConfigContext {
                cwd: Some(
                    AbsolutePathBuf::from_absolute_path_checked(
                        std::env::temp_dir().join("project"),
                    )
                    .expect("absolute cwd"),
                ),
                ..Default::default()
            })
            .await
            .expect("thread config layers load");

        assert_eq!(
            layers,
            vec![
                ConfigLayerEntry::new(
                    ConfigLayerSource::SessionFlags,
                    toml::toml! {
                        model = "gpt-user"
                    }
                    .into()
                ),
                ConfigLayerEntry::new(
                    ConfigLayerSource::SessionFlags,
                    toml::toml! {
                        model_provider = "local"

                        [model_providers.local]
                        name = "local"
                        base_url = "http://127.0.0.1:8061/api/sprite"
                        wire_api = "responses"
                        supports_websockets = true

                        [features]
                        plugins = false
                    }
                    .into()
                )
            ]
        );
    }

    fn test_provider(name: &str) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            base_url: Some("http://127.0.0.1:8061/api/sprite".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            aws: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            supports_websockets: true,
        }
    }
}
