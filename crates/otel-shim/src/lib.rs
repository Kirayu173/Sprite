use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelExporter {
    None,
    Statsig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelSettings {
    pub environment: String,
    pub service_name: String,
    pub service_version: String,
    pub codex_home: PathBuf,
    pub exporter: OtelExporter,
    pub trace_exporter: OtelExporter,
    pub metrics_exporter: OtelExporter,
    pub runtime_metrics: bool,
    pub span_attributes: BTreeMap<String, String>,
    pub tracestate: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsigMetricsSettings {
    pub environment: String,
}

#[derive(Debug, Default)]
pub struct MetricsClient;

impl MetricsClient {
    pub fn counter(&self, _name: &str, _inc: u64, _tags: &[(&str, &str)]) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct OtelProvider {
    metrics: Option<MetricsClient>,
}

impl OtelProvider {
    pub fn from(settings: &OtelSettings) -> Result<Option<Self>> {
        if settings.metrics_exporter == OtelExporter::None
            && settings.trace_exporter == OtelExporter::None
            && settings.exporter == OtelExporter::None
        {
            return Ok(None);
        }
        Ok(Some(Self {
            metrics: Some(MetricsClient),
        }))
    }

    pub fn metrics(&self) -> Option<&MetricsClient> {
        self.metrics.as_ref()
    }

    pub fn shutdown(self) {}
}

pub fn global_statsig_metrics_settings() -> Option<StatsigMetricsSettings> {
    None
}
