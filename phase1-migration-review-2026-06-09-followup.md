# Phase 1 Migration Review Follow-up - 2026-06-09

Review target: `docs/agent-runtime-migration/00-migration-order.md` phase 1.

This follow-up re-reviewed the phase 1 work after `phase1-migration-review-2026-06-09-resolution.md`.

## Findings

1. HIGH - `crates/app-protocol/src/schema_fixtures.rs:373`

   `schema_manifest_matches_generated_output` fails. `write_schema_fixtures()` can generate schema output, but the generated manifest does not match committed `crates/app-protocol/schema-manifest.txt`. This breaks the phase 1 acceptance item that schema can be generated stably, and contradicts the previous resolution claim that schema fixture regression verification was fixed.

2. MEDIUM - `crates/runtime-protocol/src/error.rs:101`, `crates/runtime-protocol/src/protocol.rs:1315`, `crates/app-protocol/src/protocol/v2/shared.rs:71`

   `CyberPolicy` remains a public runtime/app protocol error code. The previous fix renamed model verification/moderation events to `ProviderPolicy*`, but this separate public error enum still exposes upstream cyber-policy product semantics and is generated into schema.

3. MEDIUM - `crates/diagnostics/src/metrics/tags.rs:7`

   Diagnostics still exposes `AUTH_MODE_TAG = "auth_mode"` and `SessionMetricTagValues.auth_mode`. This conflicts with `17-observability-diagnostics.md` and the phase 1 diagnostics handoff claim that account/auth telemetry fields were removed.

4. MEDIUM - `crates/config/src/config_toml.rs:287`, `crates/model-provider-info/src/lib.rs:30`

   Provider config is not fully product-neutral: `openai_base_url`, reserved `OPENAI_PROVIDER_ID`, and `create_openai_provider` remain. This may be acceptable as OpenAI-compatible provider support, but it does not fully satisfy the plan's "把模型 provider schema 改为通用 provider" wording.

5. LOW - `crates/config/src/thread_config.rs:30`

   `UserThreadConfig` is empty, `NoopThreadConfigLoader` is exported, and user thread config converts to `Ok(None)`. This avoids official remote config, but the thread-config capability is only a partial phase-boundary adapter.

## Missing Requirements

- Stable app protocol schema fixture verification does not pass.
- Official/product-specific `CyberPolicy` naming is not fully removed from public protocol.
- Official account/auth telemetry fields are not fully removed from diagnostics.
- Provider schema is only partially generalized; OpenAI remains a special built-in config path.
- Thread config is present structurally but not a complete Sprite-owned loader/config capability.

## Official-Coupling Residue

- `CyberPolicy` public protocol/error code.
- `auth_mode` diagnostics metric tag.
- OpenAI-specific provider config names: `openai_base_url`, `OPENAI_PROVIDER_ID`, `create_openai_provider`.
- Legacy serde aliases remain for `model_verification`, `turn_moderation_metadata`, and `not_logged_in`; these appear compatibility-only, not active official runtime coupling.
- No active required-path residues were found for ChatGPT login, official cloud tasks, marketplace/share RPCs, attestation RPC, feedback RPC, Sentry, Statsig, or official remote thread store.

## Verification

Executed:

- `cargo check -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS, with one `app-protocol` dead-code warning.
- `cargo test -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: FAIL. `app-protocol::schema_fixtures::tests::schema_manifest_matches_generated_output` failed.
- `cargo test -p app-protocol schema_fixtures::tests::schema_manifest_matches_generated_output -- --exact --nocapture`
  - Result: FAIL, same manifest mismatch.
- `cargo test -p runtime-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS.
- `cargo test -p execpolicy -p features -p file-system -p git-utils -p network-proxy`
  - Result: PASS.
- `cargo test -p diagnostics --no-default-features`
  - Result: PASS.
- `cargo check -p diagnostics --features otlp-exporter`
  - Result: PASS.
- `cargo fmt --check`
  - Result: PASS.
- `cargo test -p config loader requirements strict_config`
  - Result: command format error, not a code failure. Full `cargo test -p config` passed in the broader run.

## Verdict

BLOCKED.

Phase 1 has migrated most core capability surfaces and most crates compile and test successfully. However, schema fixture regression verification fails, and public `CyberPolicy` plus diagnostics `auth_mode` residue remain. Fix these before declaring phase 1 complete.
