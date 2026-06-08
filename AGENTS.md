# Execution Guidelines

## 1. Think Clearly Before Coding

**Do not make assumptions by default. Do not hide confusion. Put tradeoffs on the table.**

Before implementing:

- State your assumptions clearly; ask when uncertain.
- If there are multiple possible interpretations, list them instead of silently choosing one.
- If there is a simpler approach, say so; push back when pushback is warranted.
- If something is unclear, stop first: explain what is unclear, then ask.

## 2. Choose for Long-Term Value

**Prefer the implementation that yields the optimal, longest-lasting benefit, and always consider the overall architecture before choosing an approach.**

When choosing between approaches:

- Evaluate options against the full system architecture, not just the local change site. A solution that is locally simple but creates architectural debt, coupling, or future migration cost is not actually simple.
- Prefer the option that maximizes long-term return — maintainability, extensibility, and alignment with the project's structural direction — even if it takes more effort now.
- A "simplest patch" is acceptable only when the affected surface is genuinely small, isolated, and unlikely to evolve; otherwise invest in the architecturally sound solution.
- Do not write speculative code or add unrequested features, but do not reject a more substantial change just because the immediate diff is larger, if that change is what the architecture actually needs.
- Still trim what is unnecessary: no abstractions for code used only once at the call site, no unrequested "flexibility" or "configurability", no error handling for scenarios that cannot occur.
- Before deciding, ask: "Does this fit the overall architecture, and will it still be the right shape years from now?" If not, redesign before implementing.

## 3. Make Precise Changes

**Change only what must be changed. Clean up only what you disturbed.**

When editing existing code:

- Do not "helpfully improve" adjacent code, comments, or formatting.
- Do not refactor things that are not broken.
- Match the existing style, even if your personal preference differs.
- If you find unrelated dead code, mention it briefly unless explicitly instructed; do not delete it on your own.

When your changes create unused leftovers:

- Remove imports, variables, and functions that became unused **because of your changes**.
- Do not delete pre-existing dead code unless the user explicitly asks.

The standard: **every changed line must trace directly back to the user's request.**

## 4. Keep Rust Modules Small

**Prefer focused modules over central files that keep growing.**

- Target Rust production modules under 500 lines, excluding tests.
- If a Rust file exceeds roughly 800 production lines, do not add new functionality there unless there is a strong documented reason; create or extend a focused module instead.
- When extracting code from a large module, move the related tests and module/type docs with the implementation so invariants stay close to the code that owns them.
- Do not create helper methods or modules only to satisfy line counts if they are used once and make the code harder to follow.
- Treat these high-touch files as split-first areas: `crates/runtime-core/src/app/domain/reduce.rs`, `crates/grpc/src/grpc/conversions.rs`, `apps/desktop/src-tauri/src/lib.rs`, `crates/runtime-core/src/app/domain/runtime/session_actor.rs`, and `crates/runtime-core/src/tools/builtin_tools/dispatch_agent.rs`.

## 5. Execute Toward Verifiable Goals

**Define success criteria. Iterate until verification passes.**

Turn tasks into verifiable goals, for example:

- "Add validation" -> "Write a test for invalid input, then make it pass"
- "Fix a bug" -> "Write a test that reproduces the bug, then make it pass"
- "Refactor X" -> "Tests pass before and after the refactor"

For multi-step tasks, write a short plan:

```
1. [Step] -> Verify: [check]
2. [Step] -> Verify: [check]
3. [Step] -> Verify: [check]
```

The clearer the success criteria, the easier it is to iterate independently. Weak criteria ("it runs") will keep requiring clarification.

## 6. File Encoding

**Whenever opening, reading, or writing files, default to UTF-8** unless the user or project explicitly specifies another encoding.

When reading or writing text in editors, CLI tools, and programs, use UTF-8 explicitly to avoid mojibake or silent corruption caused by system default encodings.

## 7. assistant-ui Chat Refactor Guidance

When working on the planned chat input and dialogue-area refactor, use assistant-ui as the native conversation UI layer.

- Consult `docs/assistant-ui-integration.md` and the official docs at `https://www.assistant-ui.com/llms.txt` before changing assistant-ui APIs.
- Use the installed assistant-ui skills after restarting Codex: `assistant-ui`, `setup`, `runtime`, `primitives`, `streaming`, `thread-list`, `tools`, `cloud`, and `update`.
- Wrap the chat surface with `AssistantRuntimeProvider` from `@assistant-ui/react`.
- Build the dialogue area with assistant-ui primitives such as `ThreadPrimitive`, `MessagePrimitive`, `ActionBarPrimitive`, `BranchPickerPrimitive`, and `ErrorPrimitive`.
- Build the input box with `ComposerPrimitive`; include send/cancel/edit/attachment controls only when required by the current Sprite flow.
- For Sprite's current Vite React/Tauri architecture, decide the runtime explicitly before implementation: prefer `useExternalStoreRuntime` if existing Zustand/backend state remains authoritative, `useLocalRuntime` if assistant-ui owns frontend chat state, or `useChatRuntime` only if adopting Vercel AI SDK transport.
- Do not install deprecated assistant-ui packages: `@assistant-ui/styles` or `@assistant-ui/react-ui`.

## 8. Migration: Copy-Then-Delete

Adopt a "complete-copy-then-delete" migration strategy:

1. Copy the relevant modules into the root workspace in their entirety.
2. Centrally remove official authentication, official cloud, official products, and official telemetry.
3. Use Sprite's own abstractions to replace the removed dependencies.
4. Verify by capability to ensure no core capabilities are missing.
5. Do not keep dead code from the migration. Clean up unused imports, types, functions, and modules promptly as they become unreachable; do not leave them "just in case".

Do not rewrite core state machines during the first copy. Sessions, turns, tool orchestration, persistence, and protocol models must keep their original semantics. Naming and structural convergence happen only after the official features are stripped.
