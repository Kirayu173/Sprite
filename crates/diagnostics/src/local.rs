use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

const DEFAULT_LOCAL_FILTER: &str = "info";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalLogFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub struct LocalTracingConfig {
    pub env_filter: EnvFilter,
    pub format: LocalLogFormat,
}

impl Default for LocalTracingConfig {
    fn default() -> Self {
        Self {
            env_filter: local_env_filter(),
            format: LocalLogFormat::Text,
        }
    }
}

impl LocalTracingConfig {
    pub fn json() -> Self {
        Self {
            format: LocalLogFormat::Json,
            ..Self::default()
        }
    }

    pub fn with_env_filter(mut self, env_filter: EnvFilter) -> Self {
        self.env_filter = env_filter;
        self
    }

    pub fn with_format(mut self, format: LocalLogFormat) -> Self {
        self.format = format;
        self
    }
}

pub fn local_env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(DEFAULT_LOCAL_FILTER))
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOCAL_FILTER))
}

pub fn init_local_tracing(
    config: LocalTracingConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    match config.format {
        LocalLogFormat::Text => fmt()
            .with_env_filter(config.env_filter)
            .with_writer(std::io::stderr)
            .try_init(),
        LocalLogFormat::Json => fmt()
            .json()
            .with_env_filter(config.env_filter)
            .with_writer(std::io::stderr)
            .try_init(),
    }
}

impl TryFrom<&str> for LocalTracingConfig {
    type Error = tracing_subscriber::filter::ParseError;

    fn try_from(filter: &str) -> Result<Self, Self::Error> {
        Ok(Self {
            env_filter: EnvFilter::try_new(filter)?,
            format: LocalLogFormat::Text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_tracing_config_defaults_to_text_logs() {
        let config = LocalTracingConfig::default();

        assert_eq!(config.format, LocalLogFormat::Text);
    }

    #[test]
    fn local_tracing_config_supports_json_logs() {
        let config = LocalTracingConfig::json();

        assert_eq!(config.format, LocalLogFormat::Json);
    }

    #[test]
    fn local_tracing_config_accepts_explicit_filter() {
        let config = LocalTracingConfig::try_from("diagnostics=debug").unwrap();

        assert_eq!(config.env_filter.to_string(), "diagnostics=debug");
    }
}
