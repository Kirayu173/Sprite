# Phase 1 Migration Review - 2026-06-09

Review target: `docs/agent-runtime-migration/00-migration-order.md` phase 1.

Scope reviewed:

- `AGENTS.md`
- `docs/agent-runtime-migration/00-migration-order.md`
- `docs/agent-runtime-migration/02-event-protocol-model.md`
- `docs/agent-runtime-migration/13-config-system.md`
- `docs/agent-runtime-migration/17-observability-diagnostics.md`
- `docs/agent-runtime-migration/phase1-runtime-handoff.md`
- `docs/agent-runtime-migration/phase1-config-handoff.md`
- `docs/agent-runtime-migration/phase1-diagnostics-handoff.md`
- `phase1-migration-review-current.md`
- `phase1-migration-review-current-resolution.md`
- Current root workspace phase 1 crates and support crates
- `_example/codex/codex-rs` where needed for upstream comparison

## Findings

1. HIGH - `crates/diagnostics/src/sink.rs:50`, `crates/diagnostics/src/provider.rs:100`, `crates/diagnostics/src/lib.rs:13`

   `diagnostics` has `LocalLogDiagnosticsSink` and optional OTEL provider layers, but no local tracing initializer was found that installs `tracing_subscriber::fmt`, env filtering, or JSON log formatting. This misses the `17-observability-diagnostics.md` acceptance item: ordinary local logs and JSON logs can be enabled. The current sink only emits `tracing::*` events; it does not provide a phase 1 reusable local/JSON logging setup path.

2. MEDIUM - `crates/app-protocol/src/protocol/v2/plugin.rs:267`

   Public plugin protocol still carries official marketplace/service-shaped residue: remote-only catalog wording, `PluginAuthPolicy`, `PluginInstallResponse.auth_policy`, and `PluginAvailability` accepting upstream plugin-service `"ENABLED"` at `crates/app-protocol/src/protocol/v2/plugin.rs:299` and `crates/app-protocol/src/protocol/v2/plugin.rs:311`. Marketplace/share RPCs are removed, but the public protocol is not fully Sprite-owned and product-neutral.

3. MEDIUM - `crates/app-protocol/src/schema_fixtures.rs:82`, `crates/app-protocol/src/bin/write_schema_fixtures.rs:25`

   Schema export works, but the code still describes vendored `schema/typescript` and `schema/json` fixtures while `crates/app-protocol/schema/` is absent. Phase 1's "schema can be stably generated" requirement passes by generation, but committed fixture/regression verification is missing.

4. LOW - `crates/runtime-protocol/src/protocol.rs:915`, `crates/app-protocol/src/protocol/v2/model.rs:14`

   First-party/product-specific protocol names remain: `ModelVerification`, `TrustedAccessForCyber`, `HighRiskCyberActivity`, and turn moderation metadata. These are not auth/cloud wiring, but they conflict with the protocol-neutral naming goal.

## Missing Requirements

- `17-observability-diagnostics.md`: ordinary local log and JSON log enablement is not fully migrated.
- `02-event-protocol-model.md`: protocol surface is mostly cleaned, but not fully product-neutral.
- `02-event-protocol-model.md`: schema generation is functional, but stable committed schema fixture verification is not present.

## Official-Coupling Residue

- Plugin protocol still has plugin-service/catalog/auth-policy residue.
- Model verification / cyber / moderation protocol terms remain as compatibility product semantics.
- No required runtime path was found for ChatGPT login, official cloud/backend, attestation, feedback, Sentry, Statsig, analytics, or official remote-control RPC.
- OTLP support is optional and not default-enabled; it was not counted as official telemetry residue.
- OpenAI remains only as an OpenAI-compatible provider requiring explicit `base_url`; no official account auth dependency was found.

## Verification

Executed:

- `cargo check -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS
- `cargo test -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS
- `cargo test -p execpolicy -p git-utils -p network-proxy`
  - Result: PASS
- `cargo test -p config model_catalog`
  - Result: PASS
- `cargo test -p config loader`
  - Result: PASS
- `cargo test -p config requirements`
  - Result: PASS
- `cargo test -p config strict_config`
  - Result: PASS
- `cargo test -p runtime-core diagnostics`
  - Result: PASS
- `cargo test -p diagnostics --no-default-features sink`
  - Result: PASS
- `cargo check -p diagnostics --no-default-features`
  - Result: PASS
- `cargo check -p diagnostics --features otlp-exporter`
  - Result: PASS
- `cargo run -p app-protocol --bin export -- --out target\phase1-review-current-protocol-export`
  - Result: PASS
- Exported schema official RPC scan for removed official auth/cloud/marketplace/telemetry terms
  - Result: PASS, no removed official RPC matches
- Exported schema retained-capability scan for phase 1 retained methods
  - Result: PASS, retained methods found
- `cargo fmt --check`
  - Result: PASS

One command was mistyped and failed:

- `cargo test -p config loader requirements strict_config`
  - Result: FAIL, invalid cargo argument usage
  - Follow-up: reran `loader`, `requirements`, and `strict_config` filters separately; all passed.

No source/config files were modified during review. Cargo and schema verification wrote build/export artifacts under `target`.

## Verdict

BLOCKED.

Phase 1 is close: protocol cleanup, config migration, support-crate fixes, compilation, tests, and schema export are in good shape. However, diagnostics still lacks the local/JSON tracing initialization path required by phase 1 observability acceptance, so the phase should be fixed and re-reviewed before being declared complete.
