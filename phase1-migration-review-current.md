# Phase 1 Migration Review - Current

Review target: `docs/agent-runtime-migration/00-migration-order.md` phase 1.

Reviewed:

- `AGENTS.md`
- `docs/agent-runtime-migration/00-migration-order.md`
- `docs/agent-runtime-migration/02-event-protocol-model.md`
- `docs/agent-runtime-migration/13-config-system.md`
- `docs/agent-runtime-migration/17-observability-diagnostics.md`
- `phase1-migration-review.md`
- `phase1-migration-review-resolution.md`
- Current root workspace phase 1 crates and support crates
- `_example/codex/codex-rs` where needed for upstream comparison

## Findings

1. HIGH - `crates/config/src/config_toml.rs:281`

   `model_catalog_json` is present as a config field, but the phase 1 config layer does not appear to load or apply the JSON model catalog. `13-config-system.md` requires retaining model catalog config as an effective capability, so this is currently closer to a retained field than a completed migration.

2. HIGH - `crates/execpolicy/src/lib.rs:1`

   `execpolicy` is a narrow prefix-rule slice. It lacks upstream-equivalent parser/policy/host executable/network/check_multiple/amend behavior. Because `crates/config/src/requirements_exec_policy.rs` already wires this into requirements config, this is a phase 1 config承接 risk, not only a future command-execution concern.

3. HIGH - `crates/git-utils/src/lib.rs:3`

   `resolve_root_git_project_for_trust` ignores the filesystem abstraction and directly calls host filesystem APIs. It also does not cover linked worktree `.git` files, common-dir, or root checkout handling. Since config loader uses this for project trust and `.sprite/config.toml` discovery, project config layers can be loaded incorrectly.

4. MEDIUM - `crates/network-proxy/src/lib.rs:7`

   `network-proxy` currently keeps config data structures and host normalization only. It does not retain policy decider, constraint validation, or runtime state. If phase 1 is interpreted as schema/parse only this is acceptable, but it is incomplete for the stated config goal that network/permission config can take effect.

5. MEDIUM - `crates/runtime-core/src/diagnostics/prompt_debug.rs:14`

   The previous prompt debug blocker is partially resolved: local JSON dump support exists. However, this is still an adapter boundary, not upstream-style session/history/tools/context-derived model-visible prompt assembly. This should be explicitly carried into phases 3/4.

6. MEDIUM - `crates/runtime-core/src/diagnostics/tool_dispatch_trace.rs:66`

   The runtime-core tool dispatch trace adapter covers only Function/Custom payloads and direct function output. `rollout-trace` supports broader tool dispatch shapes, including ToolSearch, LocalShell, and CodeMode-related shapes. The diagnostic foundation exists, but adapter coverage is incomplete.

## Missing Requirements

- `13-config-system.md`: model catalog config has not been proven to load and apply.
- `13-config-system.md`: execpolicy support is too narrow for requirements exec rules.
- `13-config-system.md`: git-utils support is incomplete for project config/trust/root checkout semantics.
- `13-config-system.md`: network policy config is mostly schema/parse and lacks decider/validation承接.
- `17-observability-diagnostics.md`: prompt debug and tool dispatch trace have reusable interfaces but are not fully wired to real runtime flows.
- app-server tracing has only reusable helper code. Because `crates/app-server` is not part of phase 1, this is a phase-boundary limitation rather than an independent blocker.

## Official-Coupling Residue

- No required runtime path was found for ChatGPT login, official cloud/backend, attestation, feedback, Sentry, Statsig, analytics, or official remote-control RPC.
- OpenAI remains as an ordinary OpenAI-compatible provider and requires an explicit `base_url`; no official account auth dependency was found.
- `diagnostics` retains generic OTLP exporter support behind explicit feature/config. It is not enabled by default and no official collector default was found.
- `app-protocol` still has plugin catalog/install/auth-policy terminology and remote/catalog wording. This does not currently expose official marketplace RPC, but later plugin migration must keep this Sprite-owned and not restore official marketplace coupling.
- Some compatibility names/comments remain around account/usage/managed-account/backend concepts. I did not find them forming a required official auth/cloud execution path.

## Verification

Executed:

- `cargo check -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS
- `cargo test -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - Result: PASS
- `cargo run -p app-protocol --bin export -- --out target\phase1-current-review-protocol-export`
  - Result: PASS
- Exported schema removal scan for `account/login`, `account/logout`, `account/get`, `marketplace/add`, `marketplace/remove`, `marketplace/upgrade`, `plugin/share`, `remoteControl/`, `mcpServer/oauth/login`, `attestation/generate`, `feedback/upload`, `chatgpt`, `sentry`, `statsig`, `analytics`
  - Result: PASS, no matches
- Exported schema retained-capability scan for `thread/read`, `thread/start`, `thread/resume`, `turn/start`, `turn/interrupt`, `command/exec`, `config/read`, `mcpServer/tool/call`, `skills/list`, `hooks/list`, `plugin/install`, `plugin/read`, `permissionProfile/list`
  - Result: PASS, matches found
- `cargo test -p config strict_config`
  - Result: PASS
- `cargo test -p config loader`
  - Result: PASS
- `cargo test -p diagnostics --no-default-features sink`
  - Result: PASS
- `cargo test -p diagnostics --no-default-features resource_attributes_include_host_name_when_present`
  - Result: PASS
- `cargo check -p diagnostics --features otlp-exporter`
  - Result: PASS
- Official residue scan for `Statsig|statsig|chatgpt.com|codex/analytics-events|sentry|codex_login|marketplace|attestation|feedback/upload|cloud task|cloud-task`
  - Result: PASS, no matches in reviewed phase 1 runtime paths

Not executed:

- `cargo fmt`
  - Reason: review-only task; no code changes were made during the review.

## Verdict

BLOCKED.

Phase 1 protocol cleanup and base compilation/testing are in good shape, and the previous diagnostics blockers have been materially reduced. However, the config support layer still has承接 gaps that can break later phases, especially `execpolicy`, `git-utils`, and effective model catalog loading. These should be fixed before declaring phase 1 complete.
