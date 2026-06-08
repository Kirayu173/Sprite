//! Prompt-debug integration placeholder.
//!
//! The upstream implementation depends on the full session runtime, tool
//! registry, exec server, and authentication manager. Those are intentionally
//! not reintroduced during phase 1 diagnostics migration.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDebugUnavailable {
    pub reason: &'static str,
}

pub fn build_prompt_input_unavailable() -> PromptDebugUnavailable {
    PromptDebugUnavailable {
        reason: "prompt debug requires the phase 3/4 runtime session integration",
    }
}
