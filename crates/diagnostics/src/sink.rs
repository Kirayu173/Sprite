use std::borrow::Cow;

use tracing::Level;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticsLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<DiagnosticsLevel> for Level {
    fn from(level: DiagnosticsLevel) -> Self {
        match level {
            DiagnosticsLevel::Trace => Level::TRACE,
            DiagnosticsLevel::Debug => Level::DEBUG,
            DiagnosticsLevel::Info => Level::INFO,
            DiagnosticsLevel::Warn => Level::WARN,
            DiagnosticsLevel::Error => Level::ERROR,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsEvent<'a> {
    pub level: DiagnosticsLevel,
    pub target: &'a str,
    pub message: Cow<'a, str>,
}

impl<'a> DiagnosticsEvent<'a> {
    pub fn new(level: DiagnosticsLevel, target: &'a str, message: impl Into<Cow<'a, str>>) -> Self {
        Self {
            level,
            target,
            message: message.into(),
        }
    }
}

pub trait DiagnosticsSink: Send + Sync {
    fn record(&self, event: DiagnosticsEvent<'_>);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalLogDiagnosticsSink;

impl DiagnosticsSink for LocalLogDiagnosticsSink {
    fn record(&self, event: DiagnosticsEvent<'_>) {
        match event.level {
            DiagnosticsLevel::Trace => {
                tracing::trace!(
                    target: "diagnostics.local",
                    diagnostics_target = event.target,
                    "{}",
                    event.message
                );
            }
            DiagnosticsLevel::Debug => {
                tracing::debug!(
                    target: "diagnostics.local",
                    diagnostics_target = event.target,
                    "{}",
                    event.message
                );
            }
            DiagnosticsLevel::Info => {
                tracing::info!(
                    target: "diagnostics.local",
                    diagnostics_target = event.target,
                    "{}",
                    event.message
                );
            }
            DiagnosticsLevel::Warn => {
                tracing::warn!(
                    target: "diagnostics.local",
                    diagnostics_target = event.target,
                    "{}",
                    event.message
                );
            }
            DiagnosticsLevel::Error => {
                tracing::error!(
                    target: "diagnostics.local",
                    diagnostics_target = event.target,
                    "{}",
                    event.message
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_event_stores_level_target_and_message() {
        let event =
            DiagnosticsEvent::new(DiagnosticsLevel::Info, "diagnostics.local", "local only");

        assert_eq!(event.level, DiagnosticsLevel::Info);
        assert_eq!(event.target, "diagnostics.local");
        assert_eq!(event.message, "local only");
    }

    #[test]
    fn local_log_sink_accepts_events_without_remote_exporter() {
        let sink = LocalLogDiagnosticsSink;

        sink.record(DiagnosticsEvent::new(
            DiagnosticsLevel::Debug,
            "diagnostics.local",
            "debug message",
        ));
    }
}
