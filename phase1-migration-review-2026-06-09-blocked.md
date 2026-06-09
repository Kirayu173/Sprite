# Phase 1 Migration Review - 2026-06-09

Review target: `docs/agent-runtime-migration/00-migration-order.md` phase 1.

Scope reviewed:

- `AGENTS.md`
- `docs/agent-runtime-migration/00-migration-order.md`
- `docs/agent-runtime-migration/02-event-protocol-model.md`
- `docs/agent-runtime-migration/13-config-system.md`
- `docs/agent-runtime-migration/17-observability-diagnostics.md`
- `phase1-migration-review-2026-06-09-latest.md`
- Current root workspace phase 1 code/config
- Targeted comparison with `_example/codex/codex-rs`

No files were modified during the review. This report file was created afterward at the user's request.

## Findings

1. HIGH - `crates/config/src/runtime_config.rs:319`

   `RuntimeConfig` builds its typed runtime view from raw/effective TOML but does not consistently apply `ConfigRequirements` constraints. `approval_policy`, `mcp_servers`, `hooks`, and `web_search` are copied from raw config at `runtime_config.rs:319`, `runtime_config.rs:329`, `runtime_config.rs:334`, and `runtime_config.rs:358`, while requirements define constraints for these fields at `crates/config/src/config_requirements.rs:117`.

   Impact: system/admin requirements can be loaded but bypassed in the effective typed runtime config.

2. HIGH - `crates/diagnostics/src/runtime.rs:61`, `crates/runtime-core/src/diagnostics/runtime_trace.rs:45`

   Phase 17 diagnostics are still not production-integrated. `install_runtime_diagnostics`, `RuntimeTrace::start_root`, and app-server tracing helpers exist as library APIs/tests, but there is no production runtime/app-server entrypoint wiring them into actual turn/tool execution.

   Impact: phase 17 acceptance for normal/JSON logs in production and per-thread/per-turn/per-tool trace correlation is not closed.

3. MEDIUM - `crates/config/src/runtime_config.rs:265`

   Runtime project trust resolution calls `raw.get_active_project(cwd, None)`. The loader resolves repo-root trust context at `crates/config/src/loader/mod.rs:775`, but that context is not carried into `RuntimeConfig`.

   Impact: starting in a repo subdirectory can diverge between loaded project trust and typed runtime defaults.

4. MEDIUM - `crates/runtime-protocol/src/config_types.rs:169`, `crates/app-protocol/src/protocol/v2/shared.rs:232`

   `ApprovalsReviewer::AutoReview` still serializes as legacy `guardian_subagent`. `auto_review` is accepted as an alias, but new outbound protocol/schema still exposes the legacy name as a normal value.

5. MEDIUM - `crates/model-provider-info/src/lib.rs:31`

   `model-provider-info` includes a built-in Amazon Bedrock provider with OpenAI-branded model IDs and a hardcoded cloud endpoint at `crates/model-provider-info/src/lib.rs:35`, then includes it by default at `crates/model-provider-info/src/lib.rs:409`.

   Impact: this is not official OpenAI auth/cloud coupling, but it is not a provider-neutral phase 1 base unless explicitly documented as Sprite-owned provider behavior.

6. MEDIUM - `crates/app-protocol/src/protocol/v2/turn.rs:322`

   App protocol conversion from non-exhaustive `runtime_protocol::UserInput` uses a wildcard `unreachable!`.

   Impact: a future core input variant can compile and then panic at runtime during app-protocol mapping.

## Missing Requirements

- Phase 13: effective typed runtime config does not fully enforce requirements for approval policy, MCP, hooks, and web search.
- Phase 13: repo-root keyed project trust is not preserved into typed runtime config derivation.
- Phase 17: local tracing/logging APIs exist, but production runtime/app-server turn/tool trace integration is incomplete.
- Phase 17: prompt debug and response debug context exist as helpers, but are not connected to the actual model-turn/provider path.
- Phase 02: public protocol still emits legacy `guardian_subagent` as canonical serialization.

## Official-Coupling Residue

No active required-path RPC residue was found for:

- ChatGPT login
- Official marketplace/share
- Attestation
- Feedback upload
- Remote control/cloud tasks
- Sentry
- Statsig
- Official analytics

Remaining naming/compatibility residue:

- `guardian_subagent`
- `x-oai-request-id` fallback
- OpenAI-compatible terminology
- OpenAI-branded Bedrock model IDs
- `TODO(celia-oai)`

Generated `crates/app-protocol/schema/` exists and matches the manifest, but it is currently untracked in `git status`; include it in the intended change set before submitting.

## Verification

Executed:

- `cargo metadata --no-deps --locked --format-version 1`
  - Result: PASS.
- `cargo fmt --check`
  - Result: PASS.
- Schema manifest vs `crates/app-protocol/schema` file list
  - Result: PASS, 657/657, no missing or extra files.
- Official-coupling `rg` scan over phase 1 crates
  - Result: no active blocked RPC/service residue found.
- `cargo check -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS.
- `cargo check -p runtime-core --features runtime-diagnostics`
  - Result: PASS.
- `cargo check -p diagnostics --features otlp-exporter`
  - Result: PASS.
- `cargo test -p app-protocol schema_fixtures::tests::schema_manifest_matches_generated_output -- --exact --nocapture`
  - Result: PASS.
- `cargo test -p config runtime_config -- --nocapture`
  - Result: PASS.
- `cargo test -p config thread_config -- --nocapture`
  - Result: PASS.
- `cargo test -p diagnostics --no-default-features`
  - Result: PASS.
- `cargo test -p runtime-core diagnostics --features runtime-diagnostics -- --nocapture`
  - Result: PASS.
- `cargo test -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context`
  - Result: PASS.

## Verdict

BLOCKED.

Phase 1 has substantial migration in place and the phase 1 crates compile and test successfully. However, it cannot be submitted as complete until typed `RuntimeConfig` enforces requirements and the diagnostics production-integration gap is resolved or explicitly re-scoped in the migration plan.
