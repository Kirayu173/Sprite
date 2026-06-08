//! Memory-usage diagnostics placeholder.
//!
//! The upstream hook records memory-read metrics through the tool runtime.
//! Tool execution and memories are migrated in later phases, so this module
//! only preserves the phase boundary without pulling official telemetry code.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryUsageDiagnostics;

impl MemoryUsageDiagnostics {
    pub fn record_tool_read(&self, _success: bool) {}
}
