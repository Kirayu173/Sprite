pub mod app_server_tracing;
pub(crate) mod config;
pub mod doctor;
mod local;
pub(crate) mod metrics;
pub(crate) mod provider;
mod sink;
pub(crate) mod trace_context;

#[cfg(feature = "otlp-exporter")]
mod otlp;
mod targets;

pub use crate::config::OtelExporter;
pub use crate::config::OtelHttpProtocol;
pub use crate::config::OtelSettings;
pub use crate::config::OtelTlsConfig;
pub use crate::config::validate_span_attributes;
pub use crate::local::LocalLogFormat;
pub use crate::local::LocalTracingConfig;
pub use crate::local::init_local_tracing;
pub use crate::local::local_env_filter;
use crate::metrics::Result as MetricsResult;
pub use crate::metrics::global as global_metrics;
pub use crate::metrics::runtime_metrics::RuntimeMetricTotals;
pub use crate::metrics::runtime_metrics::RuntimeMetricsSummary;
pub use crate::metrics::timer::Timer;
pub use crate::metrics::*;
pub use crate::provider::OtelProvider;
pub use crate::sink::DiagnosticsEvent;
pub use crate::sink::DiagnosticsLevel;
pub use crate::sink::DiagnosticsSink;
pub use crate::sink::LocalLogDiagnosticsSink;
pub use crate::trace_context::context_from_w3c_trace_context;
pub use crate::trace_context::current_span_trace_id;
pub use crate::trace_context::current_span_w3c_trace_context;
pub use crate::trace_context::set_parent_from_context;
pub use crate::trace_context::set_parent_from_w3c_trace_context;
pub use crate::trace_context::span_w3c_trace_context;
pub use crate::trace_context::traceparent_context_from_env;
pub use crate::trace_context::validate_tracestate_entries;
pub use crate::trace_context::validate_tracestate_member;
pub use utils_string::sanitize_metric_tag_value;

/// Start a metrics timer using the globally installed metrics client.
pub fn start_global_timer(name: &str, tags: &[(&str, &str)]) -> MetricsResult<Timer> {
    let Some(metrics) = crate::metrics::global() else {
        return Err(MetricsError::ExporterDisabled);
    };
    metrics.start_timer(name, tags)
}
