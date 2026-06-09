//! Memory-usage diagnostics for shell-like read commands.

use std::io;

#[cfg(feature = "runtime-diagnostics")]
use diagnostics::MEMORIES_USAGE_METRIC;
#[cfg(feature = "runtime-diagnostics")]
use diagnostics::MetricsClient;
#[cfg(feature = "runtime-diagnostics")]
use diagnostics::global_metrics;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct MemoryUsageDiagnostics {
    #[cfg(feature = "runtime-diagnostics")]
    metrics: Option<MetricsClient>,
}

impl MemoryUsageDiagnostics {
    pub fn global() -> Self {
        #[cfg(feature = "runtime-diagnostics")]
        return Self {
            metrics: global_metrics(),
        };
        #[cfg(not(feature = "runtime-diagnostics"))]
        return Self {};
    }

    #[cfg(feature = "runtime-diagnostics")]
    pub fn with_metrics(metrics: MetricsClient) -> Self {
        Self {
            metrics: Some(metrics),
        }
    }

    pub fn disabled() -> Self {
        #[cfg(feature = "runtime-diagnostics")]
        return Self { metrics: None };
        #[cfg(not(feature = "runtime-diagnostics"))]
        return Self {};
    }

    pub fn record_shell_command(&self, tool_name: &str, command: &[String], success: bool) {
        #[cfg(feature = "runtime-diagnostics")]
        {
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
        #[cfg(not(feature = "runtime-diagnostics"))]
        {
            let _ = (tool_name, command, success);
        }
    }

    pub fn snapshot_current_process(&self) -> io::Result<MemoryUsageSnapshot> {
        MemoryUsageSnapshot::current_process()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MemoryUsageSnapshot {
    pub process_id: u32,
    pub resident_set_bytes: u64,
    pub virtual_memory_bytes: Option<u64>,
    pub peak_resident_set_bytes: Option<u64>,
}

impl MemoryUsageSnapshot {
    pub fn current_process() -> io::Result<Self> {
        platform_memory_usage_snapshot()
    }

    pub fn to_json_pretty(&self) -> io::Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

#[cfg(windows)]
fn platform_memory_usage_snapshot() -> io::Result<MemoryUsageSnapshot> {
    use std::mem::size_of;
    use windows_sys::Win32::System::ProcessStatus::GetProcessMemoryInfo;
    use windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };

    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(MemoryUsageSnapshot {
        process_id: std::process::id(),
        resident_set_bytes: counters.WorkingSetSize as u64,
        virtual_memory_bytes: Some(counters.PagefileUsage as u64),
        peak_resident_set_bytes: Some(counters.PeakWorkingSetSize as u64),
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_memory_usage_snapshot() -> io::Result<MemoryUsageSnapshot> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let resident_set_bytes = proc_status_kib_value(&status, "VmRSS")
        .map(|value| value.saturating_mul(1024))
        .ok_or_else(|| io::Error::other("failed to read VmRSS from /proc/self/status"))?;
    let virtual_memory_bytes =
        proc_status_kib_value(&status, "VmSize").map(|value| value.saturating_mul(1024));
    let peak_resident_set_bytes =
        proc_status_kib_value(&status, "VmHWM").map(|value| value.saturating_mul(1024));

    Ok(MemoryUsageSnapshot {
        process_id: std::process::id(),
        resident_set_bytes,
        virtual_memory_bytes,
        peak_resident_set_bytes,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn proc_status_kib_value(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name != key {
            return None;
        }
        value.split_whitespace().next()?.parse().ok()
    })
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn platform_memory_usage_snapshot() -> io::Result<MemoryUsageSnapshot> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "memory usage snapshots are not supported on this platform yet",
    ))
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

    #[test]
    fn snapshots_current_process_memory_usage() {
        let snapshot = MemoryUsageSnapshot::current_process().unwrap();

        assert_eq!(snapshot.process_id, std::process::id());
        assert!(snapshot.resident_set_bytes > 0);
        assert!(
            snapshot
                .to_json_pretty()
                .unwrap()
                .contains("resident_set_bytes")
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn parses_proc_status_memory_values() {
        let status = "VmSize:\t  123 kB\nVmRSS:\t  45 kB\nVmHWM:\t  67 kB\n";

        assert_eq!(proc_status_kib_value(status, "VmRSS"), Some(45));
        assert_eq!(proc_status_kib_value(status, "VmHWM"), Some(67));
        assert_eq!(proc_status_kib_value(status, "Missing"), None);
    }
}
