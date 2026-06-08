//! Runtime-core adapter placeholder for rollout-trace tool dispatch events.
//!
//! The trace schema and no-op-capable writer live in `rollout-trace`. The
//! concrete adapter from runtime tool invocations is deferred until phase 5,
//! when the Sprite tool registry exists in this workspace.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDispatchTraceAdapterUnavailable;
