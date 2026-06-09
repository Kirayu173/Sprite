use crate::LocalLogFormat;
use crate::LocalTracingConfig;
use crate::OtelProvider;
use crate::OtelSettings;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

#[derive(Debug, Clone)]
pub struct RuntimeDiagnosticsConfig {
    pub local_log_format: LocalLogFormat,
    pub env_filter: EnvFilter,
    pub otel: Option<OtelSettings>,
}

impl Default for RuntimeDiagnosticsConfig {
    fn default() -> Self {
        let local = LocalTracingConfig::default();
        Self {
            local_log_format: local.format,
            env_filter: local.env_filter,
            otel: None,
        }
    }
}

impl RuntimeDiagnosticsConfig {
    pub fn json_logs() -> Self {
        let local = LocalTracingConfig::json();
        Self {
            local_log_format: local.format,
            env_filter: local.env_filter,
            otel: None,
        }
    }

    pub fn with_otel(mut self, otel: OtelSettings) -> Self {
        self.otel = Some(otel);
        self
    }
}

pub struct RuntimeDiagnosticsGuard {
    provider: Option<OtelProvider>,
}

impl RuntimeDiagnosticsGuard {
    pub fn provider(&self) -> Option<&OtelProvider> {
        self.provider.as_ref()
    }
}

impl Drop for RuntimeDiagnosticsGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.as_ref() {
            provider.shutdown();
        }
    }
}

pub fn install_runtime_diagnostics(
    config: RuntimeDiagnosticsConfig,
) -> Result<RuntimeDiagnosticsGuard, Box<dyn std::error::Error>> {
    let provider = match config.otel.as_ref() {
        Some(otel) => OtelProvider::from(otel)?,
        None => None,
    };

    match (config.local_log_format, provider.as_ref()) {
        (LocalLogFormat::Text, Some(provider)) => tracing_subscriber::registry()
            .with(config.env_filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .with(provider.tracing_layer())
            .with(provider.logger_layer())
            .try_init()?,
        (LocalLogFormat::Json, Some(provider)) => tracing_subscriber::registry()
            .with(config.env_filter)
            .with(fmt::layer().json().with_writer(std::io::stderr))
            .with(provider.tracing_layer())
            .with(provider.logger_layer())
            .try_init()?,
        (LocalLogFormat::Text, None) => tracing_subscriber::registry()
            .with(config.env_filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .try_init()?,
        (LocalLogFormat::Json, None) => tracing_subscriber::registry()
            .with(config.env_filter)
            .with(fmt::layer().json().with_writer(std::io::stderr))
            .try_init()?,
    }

    Ok(RuntimeDiagnosticsGuard { provider })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_diagnostics_defaults_to_local_text_without_otel() {
        let config = RuntimeDiagnosticsConfig::default();

        assert_eq!(config.local_log_format, LocalLogFormat::Text);
        assert!(config.otel.is_none());
    }

    #[test]
    fn runtime_diagnostics_supports_json_logs_without_otel() {
        let config = RuntimeDiagnosticsConfig::json_logs();

        assert_eq!(config.local_log_format, LocalLogFormat::Json);
        assert!(config.otel.is_none());
    }
}
