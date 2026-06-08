# Phase 1 Migration Review Resolution

Source review: `phase1-migration-review.md`.

## Resolved Findings

1. `prompt_debug` lacked local prompt debug dump support.

   Status: fixed.

   `crates/runtime-core/src/diagnostics/prompt_debug.rs` now exposes:

   - `PromptDebugInput` with history, current user input, base instructions, and dynamic tool metadata.
   - `PromptDebugDump` as the local model-visible request debug shape.
   - `write_prompt_debug_dump` for local JSON output.

   Phase 1 does not yet contain the full session runtime, so this code does not pretend to start a thread/session. It provides the reusable debug assembly boundary that later session code can call with Sprite-owned session history, base instructions, and tool metadata.

2. `memory_usage` lacked a memory usage snapshot.

   Status: fixed.

   `crates/runtime-core/src/diagnostics/memory_usage.rs` now keeps the existing tool-read metric helper and adds `MemoryUsageSnapshot::current_process()` plus `MemoryUsageDiagnostics::snapshot_current_process()`. Windows uses process memory counters; Linux-like `/proc` platforms read `/proc/self/status`; unsupported platforms return an explicit `Unsupported` error instead of fake data.

3. `runtime-core` had crate-wide `#![allow(dead_code)]`.

   Status: fixed.

   The crate-wide allow was removed. `runtime_core::config_lock` and `runtime_core::diagnostics` are now public modules so phase 1 diagnostics/config-lock adapters are visible to downstream runtime, tool, and app-server integration code.

## Misjudged Or Boundary-Limited Items

1. Explicit OTLP support in `crates/diagnostics`.

   Status: not a defect.

   The phase 1 handoff intentionally retains generic, user-configured OTLP exporters behind the default-off `otlp-exporter` feature. This is not an official hosted collector, official analytics, Sentry, account telemetry, or product telemetry path. With default features disabled or unset exporters, diagnostics remains local-only.

2. `app-server tracing` is not fully integrated in phase 1.

   Status: phase-boundary limitation, not a missing phase 1 workspace crate.

   The phase 1 workspace has no `crates/app-server` member. `diagnostics::app_server_tracing` is the reusable helper prepared for phase 7, when app-server is migrated. Full app-server runtime wiring cannot be completed before that crate exists.

3. Full session-derived prompt assembly cannot be completed in phase 1.

   Status: phase-boundary limitation now reduced to an explicit adapter boundary.

   The upstream Codex `prompt_debug` starts a real session and calls `build_prompt`. Sprite phase 1 has not migrated session runtime, context assembly, model runtime, or tool registry yet. The current fix avoids adding official auth/cloud/runtime stubs and instead exposes the local dump shape and assembly inputs that the later Sprite session runtime will provide.

## Verification

- `cargo check -p runtime-core`
- `cargo test -p runtime-core prompt_debug`
- `cargo test -p runtime-core memory_usage`

