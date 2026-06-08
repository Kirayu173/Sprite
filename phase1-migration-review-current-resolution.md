# Phase 1 Migration Review Current Resolution

Review target: `phase1-migration-review-current.md`

## Finding Resolutions

1. HIGH - `model_catalog_json`
   - Status: confirmed real gap.
   - Resolution: `ConfigLayerStack::load_model_catalog_json` now loads the effective `model_catalog_json` path as `runtime_protocol::model_capabilities::ModelsResponse`, rejects invalid JSON, and rejects empty catalogs.
   - Verification: `cargo test -p config model_catalog`.

2. HIGH - `execpolicy`
   - Status: confirmed real gap.
   - Resolution: `crates/execpolicy` now restores the phase 1 policy surface: prefix rules, network rules, host executable matching, `check_multiple`, match options, amend helpers, and a local `.rules` parser for `prefix_rule`, `network_rule`, and `host_executable`.
   - Note: the parser intentionally avoids the upstream `starlark` crate because that dependency does not compile in this workspace dependency graph. The retained policy language covers the migrated phase 1 builtins without restoring official auth/cloud/telemetry.
   - Verification: `cargo test -p execpolicy`.

3. HIGH - `git-utils`
   - Status: confirmed real gap.
   - Resolution: `resolve_root_git_project_for_trust` now uses `ExecutorFileSystem`, supports regular `.git` directories, nested paths, linked worktree `.git` files, `.git/worktrees/<name>` common-dir resolution, and rejects non-worktree gitdir files.
   - Verification: `cargo test -p git-utils`.

4. MEDIUM - `network-proxy`
   - Status: confirmed phase 1 policy gap, not a request to migrate the full proxy server.
   - Resolution: `network-proxy` now has host normalization parity, domain glob allow/deny compilation, limited/full method policy, loopback/non-public IP helpers, unix socket allowlist validation, managed constraint validation, and lightweight runtime host decision state. `config` can convert network requirements into the proxy validator shape.
   - Verification: `cargo test -p network-proxy` and `cargo test -p config network_constraints_convert_to_proxy_validator`.

5. MEDIUM - prompt debug
   - Status: phase-boundary limitation, not a remaining phase 1 blocker.
   - Resolution: current phase 1 keeps reusable local prompt debug dump helpers. Full session/history/tools/context-derived model-visible prompt assembly belongs to phases 3/4, when context assembly and session runtime exist.
   - Verification: covered by `runtime-core` diagnostics tests.

6. MEDIUM - tool dispatch trace adapter
   - Status: confirmed adapter coverage gap.
   - Resolution: runtime-core adapter now covers Function, ToolSearch, Custom, LocalShell payloads plus direct responses, code-mode responses, completion, and failure.
   - Verification: `cargo test -p runtime-core diagnostics`.

## Phase Boundary Notes

- App-server tracing remains a phase-boundary limitation because `crates/app-server` is not part of phase 1.
- The full network HTTP/SOCKS/MITM proxy server remains outside phase 1. Phase 1 now owns schema, policy, validation, and decision state so later execution phases can enforce the config.
- No official ChatGPT login, official cloud/backend, official marketplace RPC, Sentry, Statsig, analytics, feedback, or attestation path was intentionally restored.

## Final Verification

Executed:

- `cargo test -p config model_catalog`: PASS
- `cargo test -p config loader`: PASS
- `cargo test -p config requirements`: PASS
- `cargo test -p execpolicy`: PASS
- `cargo test -p git-utils`: PASS
- `cargo test -p network-proxy`: PASS
- `cargo test -p runtime-core diagnostics`: PASS
- `cargo check -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`: PASS
- `cargo test -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`: PASS
- `cargo run -p app-protocol --bin export -- --out target\phase1-current-review-protocol-export`: PASS
- Exported schema removal scan for official auth/cloud/marketplace/telemetry terms: PASS, no matches.
- Exported schema retained-capability scan for phase 1 retained methods: PASS, matches found.
- Source residue scan for official auth/cloud/telemetry terms: PASS. A broader earlier scan matched `CollabAgentStatusEntry` only because the substring `sEntry` contains `sentry`; the word-boundary `\bsentry\b` scan has no matches.
- `cargo fmt --check`: PASS
