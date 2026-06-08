# 阶段 1 迁移审查报告

审查对象：`docs/agent-runtime-migration/00-migration-order.md` 中【阶段 1：协议和配置基座】。

审查范围包括：

- `AGENTS.md`
- `docs/agent-runtime-migration/00-migration-order.md`
- 阶段 1 对应方案：
  - `docs/agent-runtime-migration/02-event-protocol-model.md`
  - `docs/agent-runtime-migration/13-config-system.md`
  - `docs/agent-runtime-migration/17-observability-diagnostics.md`
- 阶段 1 handoff 文档：
  - `docs/agent-runtime-migration/phase1-runtime-handoff.md`
  - `docs/agent-runtime-migration/phase1-config-handoff.md`
  - `docs/agent-runtime-migration/phase1-diagnostics-handoff.md`
- 当前根目录 Rust workspace 中阶段 1 已迁移代码和配置
- 必要位置对照 `_example/codex/codex-rs` 原始实现

## Findings

1. BLOCKER - `crates/runtime-core/src/diagnostics/prompt_debug.rs:13`

   `prompt_debug` 不是阶段 1 要求能力的真实迁移。原始实现 `_example/codex/codex-rs/core/src/prompt_debug.rs` 会通过 session、history、tools、base instructions 和 `build_prompt` 构造真实的 model-visible prompt；当前迁移代码只把 `UserInput` 追加到 `existing_items`，没有本地 prompt debug dump，也没有真实 prompt assembly。  
   这不满足 `17-observability-diagnostics.md` 中 “prompt debug dump / prompt debug 可本地输出” 的验收要求。

2. BLOCKER - `crates/runtime-core/src/diagnostics/memory_usage.rs:29`

   memory diagnostics 被降级为 shell 命令分类和指标计数。阶段 17 要求保留 “memory usage snapshot”，但当前没有 snapshot API、没有进程或运行时内存采样，也没有对应诊断输出。  
   这属于能力缺口，不只是命名或结构差异。

3. HIGH - `crates/runtime-core/src/lib.rs:1`

   `#![allow(dead_code)]` 会掩盖阶段 1 runtime-core 诊断/config-lock 代码未被接入的问题。迁移审查中，这会降低编译验证的有效性，因为未完成或未调用的 adapter 可以长期静默存在。结合 handoff 中曾把部分 runtime-core diagnostics 描述为 placeholder，这里应作为风险处理。

4. MEDIUM - `crates/diagnostics/src/config.rs:54` 和 `crates/diagnostics/Cargo.toml:17`

   diagnostics 仍然公开 `OtelExporter::{None,OtlpHttp,OtlpGrpc}`，并保留 `otlp-exporter` feature。审查中未发现官方 hosted collector 或默认远程 exporter，因此这不等同于官方遥测残留；但它与阶段验收中的 “diagnostics 只输出本地日志” 严格表述不完全一致。  
   如果 Sprite 需要保留显式配置的通用 OTLP，应在阶段 1 文档中明确作为例外，否则容易被误判为遥测能力未剥离。

## Missing Requirements

- 阶段 17 的 “prompt debug dump / prompt debug 可本地输出” 未完整满足。
- 阶段 17 的 “memory usage snapshot” 未实现。
- 阶段 17 要求 diagnostics 可被 runtime、app-server、工具和模型层调用；当前只具备部分基础设施，runtime-core 诊断模块尚未接入真实 runtime 流程。
- `app-server` 本身不在阶段 1 workspace 迁移范围内，因此 `app-server tracing` 只能算准备了 reusable helper，不能算完整集成。
- 本次审查未执行 `cargo fmt`，只执行了编译、测试、schema 导出和残留扫描。

## Official-Coupling Residue

- 未发现公开 app-protocol RPC 中残留以下官方能力：
  - account login/logout/read
  - marketplace add/remove/upgrade
  - plugin share
  - remote control
  - MCP OAuth login
  - attestation
  - feedback
  - ChatGPT auth token refresh
- `crates/runtime-protocol/src/account.rs` 和 `crates/runtime-protocol/src/auth.rs` 已通用化为 provider account/auth error，不是 ChatGPT 官方账号模型。
- `crates/model-provider-info` 中仍保留 OpenAI-compatible provider，但它要求显式 `base_url`，没有官方认证依赖；这符合阶段文档允许的普通 OpenAI-compatible provider。
- 仍有兼容性命名、注释或测试样例中的 OpenAI/Codex 字样，但未发现它们构成必需运行路径中的官方认证、官方云、marketplace、attestation、feedback、Sentry 或 analytics 耦合。

## Verification

已执行：

- `cargo check -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - 结果：PASS
- `cargo test -p runtime-protocol -p app-protocol -p model-provider-info -p config -p diagnostics -p rollout-trace -p response-debug-context -p runtime-core`
  - 结果：PASS
- `cargo run -p app-protocol --bin export -- --out target\phase1-review-protocol-export`
  - 结果：PASS
- `rg` 检查导出 schema 中已删除官方 RPC：
  - `account/login`
  - `account/logout`
  - `account/get`
  - `marketplace/add`
  - `marketplace/remove`
  - `marketplace/upgrade`
  - `plugin/share`
  - `remoteControl/`
  - `mcpServer/oauth/login`
  - `attestation/generate`
  - `feedback/upload`
  - `chatgpt`
  - 结果：PASS，无命中
- `rg` 检查导出 schema 中保留核心 RPC：
  - `thread/read`
  - `thread/start`
  - `thread/resume`
  - `turn/start`
  - `turn/interrupt`
  - `command/exec`
  - `config/read`
  - `mcpServer/tool/call`
  - `skills/list`
  - `hooks/list`
  - `plugin/install`
  - `plugin/read`
  - `permissionProfile/list`
  - 结果：PASS，有命中
- `cargo test -p config strict_config`
  - 结果：PASS
- `cargo test -p config loader`
  - 结果：PASS
- `cargo test -p diagnostics --no-default-features sink`
  - 结果：PASS
- `cargo test -p diagnostics --no-default-features resource_attributes_include_host_name_when_present`
  - 结果：PASS

执行失败但已修正重跑：

- `cargo test -p config strict_config loader`
  - 结果：失败，原因是 Cargo 不接受多个 test filter 作为并列参数；已拆分为单 filter 重跑并通过。
- `cargo test -p diagnostics --no-default-features sink provider::tests::resource_attributes_include_host_name_when_present`
  - 结果：失败，原因同上；已拆分为单 filter 重跑并通过。

未执行：

- `cargo fmt`
  - 原因：本次是审查任务，未修改代码；且用户要求只审查，不做修复。

## Verdict

BLOCKED：阶段 1 的协议和配置部分整体接近完成，公开协议面也已剥离主要官方 RPC；但阶段 17 的 diagnostics 仍存在关键能力缺口，尤其是 prompt debug 和 memory usage snapshot。  

这些问题修复前，不建议判定【阶段 1】已完成或可以提交。
