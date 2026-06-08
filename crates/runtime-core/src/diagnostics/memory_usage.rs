//! Memory-usage diagnostics for shell-like read commands.

use diagnostics::MEMORIES_USAGE_METRIC;
use diagnostics::MetricsClient;
use diagnostics::global_metrics;

#[derive(Debug, Clone)]
pub struct MemoryUsageDiagnostics {
    metrics: Option<MetricsClient>,
}

impl MemoryUsageDiagnostics {
    pub fn global() -> Self {
        Self {
            metrics: global_metrics(),
        }
    }

    pub fn with_metrics(metrics: MetricsClient) -> Self {
        Self {
            metrics: Some(metrics),
        }
    }

    pub fn disabled() -> Self {
        Self { metrics: None }
    }

    pub fn record_shell_command(&self, tool_name: &str, command: &[String], success: bool) {
        let Some(metrics) = &self.metrics else {
            return;
        };
        for kind in memory_usage_kinds_from_command(command) {
            let _ = metrics.counter(
                MEMORIES_USAGE_METRIC,
                1,
                &[
                    ("kind", kind.as_str()),
                    ("tool", tool_name),
                    ("success", if success { "true" } else { "false" }),
                ],
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryUsageKind {
    ReadFile,
    Search,
    ListDirectory,
}

impl MemoryUsageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::Search => "search",
            Self::ListDirectory => "list_directory",
        }
    }
}

pub fn memory_usage_kinds_from_command(command: &[String]) -> Vec<MemoryUsageKind> {
    let Some(program) = command.first().map(|value| value.as_str()) else {
        return Vec::new();
    };
    match program {
        "cat" | "bat" | "batcat" | "type" | "Get-Content" => vec![MemoryUsageKind::ReadFile],
        "grep" | "rg" | "ripgrep" | "Select-String" => vec![MemoryUsageKind::Search],
        "ls" | "dir" | "Get-ChildItem" => vec![MemoryUsageKind::ListDirectory],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_memory_read_commands() {
        assert_eq!(
            memory_usage_kinds_from_command(&["rg".to_string(), "needle".to_string()]),
            vec![MemoryUsageKind::Search]
        );
        assert_eq!(
            memory_usage_kinds_from_command(&["cat".to_string(), "AGENTS.md".to_string()]),
            vec![MemoryUsageKind::ReadFile]
        );
        assert_eq!(
            memory_usage_kinds_from_command(&["echo".to_string(), "hello".to_string()]),
            Vec::new()
        );
    }
}
