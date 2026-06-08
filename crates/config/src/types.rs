//! Types used to define loaded and effective Sprite configuration values.

// Note this file should generally be restricted to simple struct/enum
// definitions that do not contain business logic.

pub use crate::mcp_types::AppToolApproval;
pub use crate::mcp_types::McpServerConfig;
pub use crate::mcp_types::McpServerDisabledReason;
pub use crate::mcp_types::McpServerEnvVar;
pub use crate::mcp_types::McpServerOAuthConfig;
pub use crate::mcp_types::McpServerToolConfig;
pub use crate::mcp_types::McpServerTransportConfig;
pub use crate::mcp_types::RawMcpServerConfig;
pub use runtime_protocol::config_types::AltScreenMode;
pub use runtime_protocol::config_types::ApprovalsReviewer;
use runtime_protocol::config_types::EnvironmentVariablePattern;
pub use runtime_protocol::config_types::ModeKind;
pub use runtime_protocol::config_types::Personality;
pub use runtime_protocol::config_types::ServiceTier;
use runtime_protocol::config_types::ShellEnvironmentPolicy;
use runtime_protocol::config_types::ShellEnvironmentPolicyInherit;
pub use runtime_protocol::config_types::WebSearchMode;
use std::collections::HashMap;
use std::fmt;
use utils_absolute_path::AbsolutePathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

pub use crate::tui_keymap::KeybindingSpec;
pub use crate::tui_keymap::KeybindingsSpec;
pub use crate::tui_keymap::MAX_FUNCTION_KEY;
pub use crate::tui_keymap::TuiApprovalKeymap;
pub use crate::tui_keymap::TuiChatKeymap;
pub use crate::tui_keymap::TuiComposerKeymap;
pub use crate::tui_keymap::TuiEditorKeymap;
pub use crate::tui_keymap::TuiGlobalKeymap;
pub use crate::tui_keymap::TuiKeymap;
pub use crate::tui_keymap::TuiListKeymap;
pub use crate::tui_keymap::TuiPagerKeymap;
pub use crate::tui_keymap::TuiVimNormalKeymap;
pub use crate::tui_keymap::TuiVimOperatorKeymap;

pub const DEFAULT_MEMORIES_MAX_ROLLOUTS_PER_STARTUP: usize = 2;
pub const DEFAULT_MEMORIES_MAX_ROLLOUT_AGE_DAYS: i64 = 10;
pub const DEFAULT_MEMORIES_MIN_ROLLOUT_IDLE_HOURS: i64 = 6;
pub const DEFAULT_MEMORIES_MIN_RATE_LIMIT_REMAINING_PERCENT: i64 = 25;
pub const DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION: usize = 256;
pub const DEFAULT_MEMORIES_MAX_UNUSED_DAYS: i64 = 30;
const MIN_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION: usize = 1;
const MAX_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION: usize = 4096;
const MIN_MEMORIES_MAX_ROLLOUTS_PER_STARTUP: usize = 1;
const MAX_MEMORIES_MAX_ROLLOUTS_PER_STARTUP: usize = 128;

/// Preferred layout for the resume/fork session picker.
#[derive(Serialize, Deserialize, Debug, Default, Copy, Clone, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SessionPickerViewMode {
    Comfortable,
    #[default]
    Dense,
}

impl SessionPickerViewMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Dense => "dense",
        }
    }
}

impl fmt::Display for SessionPickerViewMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Determine where Sprite should store and read MCP credentials.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OAuthCredentialsStoreMode {
    /// `Keyring` when available; otherwise, `File`.
    /// Credentials stored in the keyring will only be readable by Sprite unless the user explicitly grants access via OS-level keyring access.
    #[default]
    Auto,
    /// SPRITE_HOME/.credentials.json
    /// This file will be readable to Sprite and other applications running as the same user.
    File,
    /// Keyring when available, otherwise fail.
    Keyring,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsSandboxModeToml {
    Elevated,
    Unelevated,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct WindowsToml {
    pub sandbox: Option<WindowsSandboxModeToml>,
    /// Defaults to `true`. Set to `false` to launch the final sandboxed child
    /// process on `Winsta0\\Default` instead of a private desktop.
    pub sandbox_private_desktop: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, JsonSchema)]
pub enum UriBasedFileOpener {
    #[serde(rename = "vscode")]
    VsCode,

    #[serde(rename = "vscode-insiders")]
    VsCodeInsiders,

    #[serde(rename = "windsurf")]
    Windsurf,

    #[serde(rename = "cursor")]
    Cursor,

    /// Option to disable the URI-based file opener.
    #[serde(rename = "none")]
    None,
}

impl UriBasedFileOpener {
    pub fn get_scheme(&self) -> Option<&str> {
        match self {
            UriBasedFileOpener::VsCode => Some("vscode"),
            UriBasedFileOpener::VsCodeInsiders => Some("vscode-insiders"),
            UriBasedFileOpener::Windsurf => Some("windsurf"),
            UriBasedFileOpener::Cursor => Some("cursor"),
            UriBasedFileOpener::None => None,
        }
    }
}

/// Settings that govern if and what will be written to `~/.sprite/history.jsonl`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[serde(default)]
#[schemars(deny_unknown_fields)]
pub struct History {
    /// If true, history entries will not be written to disk.
    pub persistence: HistoryPersistence,

    /// If set, the maximum size of the history file in bytes. The oldest entries
    /// are dropped once the file exceeds this limit.
    pub max_bytes: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Default, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryPersistence {
    /// Save all history entries to disk.
    #[default]
    SaveAll,
    /// Do not write history to disk.
    None,
}

/// Memories settings loaded from config.toml.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct MemoriesToml {
    /// When `true`, external context sources mark the thread `memory_mode` as `"polluted"`.
    #[serde(alias = "no_memories_if_mcp_or_web_search")]
    pub disable_on_external_context: Option<bool>,
    /// When `false`, newly created threads are stored with `memory_mode = "disabled"` in the state DB.
    pub generate_memories: Option<bool>,
    /// When `false`, skip injecting memory usage instructions into developer prompts.
    pub use_memories: Option<bool>,
    /// When `true`, expose dedicated memory tools through the extension tool surface.
    pub dedicated_tools: Option<bool>,
    /// Maximum number of recent raw memories retained for global consolidation.
    #[schemars(range(min = 1, max = 4096))]
    pub max_raw_memories_for_consolidation: Option<usize>,
    /// Maximum number of days since a memory was last used before it becomes ineligible for phase 2 selection.
    pub max_unused_days: Option<i64>,
    /// Maximum age of the threads used for memories.
    pub max_rollout_age_days: Option<i64>,
    /// Maximum number of rollout candidates processed per pass.
    #[schemars(range(min = 1, max = 128))]
    pub max_rollouts_per_startup: Option<usize>,
    /// Minimum idle time between last thread activity and memory creation (hours). > 12h recommended.
    pub min_rollout_idle_hours: Option<i64>,
    /// Minimum remaining percentage required in provider rate-limit windows before memory startup runs.
    #[schemars(range(min = 0, max = 100))]
    pub min_rate_limit_remaining_percent: Option<i64>,
    /// Model used for thread summarisation.
    pub extract_model: Option<String>,
    /// Model used for memory consolidation.
    pub consolidation_model: Option<String>,
}

/// Effective memories settings after defaults are applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoriesConfig {
    pub disable_on_external_context: bool,
    pub generate_memories: bool,
    pub use_memories: bool,
    pub dedicated_tools: bool,
    pub max_raw_memories_for_consolidation: usize,
    pub max_unused_days: i64,
    pub max_rollout_age_days: i64,
    pub max_rollouts_per_startup: usize,
    pub min_rollout_idle_hours: i64,
    pub min_rate_limit_remaining_percent: i64,
    pub extract_model: Option<String>,
    pub consolidation_model: Option<String>,
}

impl Default for MemoriesConfig {
    fn default() -> Self {
        Self {
            disable_on_external_context: false,
            generate_memories: true,
            use_memories: true,
            dedicated_tools: false,
            max_raw_memories_for_consolidation: DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
            max_unused_days: DEFAULT_MEMORIES_MAX_UNUSED_DAYS,
            max_rollout_age_days: DEFAULT_MEMORIES_MAX_ROLLOUT_AGE_DAYS,
            max_rollouts_per_startup: DEFAULT_MEMORIES_MAX_ROLLOUTS_PER_STARTUP,
            min_rollout_idle_hours: DEFAULT_MEMORIES_MIN_ROLLOUT_IDLE_HOURS,
            min_rate_limit_remaining_percent: DEFAULT_MEMORIES_MIN_RATE_LIMIT_REMAINING_PERCENT,
            extract_model: None,
            consolidation_model: None,
        }
    }
}

impl From<MemoriesToml> for MemoriesConfig {
    fn from(toml: MemoriesToml) -> Self {
        let defaults = Self::default();
        Self {
            disable_on_external_context: toml
                .disable_on_external_context
                .unwrap_or(defaults.disable_on_external_context),
            generate_memories: toml.generate_memories.unwrap_or(defaults.generate_memories),
            use_memories: toml.use_memories.unwrap_or(defaults.use_memories),
            dedicated_tools: toml.dedicated_tools.unwrap_or(defaults.dedicated_tools),
            max_raw_memories_for_consolidation: toml
                .max_raw_memories_for_consolidation
                .unwrap_or(defaults.max_raw_memories_for_consolidation)
                .clamp(
                    MIN_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
                    MAX_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION,
                ),
            max_unused_days: toml
                .max_unused_days
                .unwrap_or(defaults.max_unused_days)
                .clamp(0, 365),
            max_rollout_age_days: toml
                .max_rollout_age_days
                .unwrap_or(defaults.max_rollout_age_days)
                .clamp(0, 90),
            max_rollouts_per_startup: toml
                .max_rollouts_per_startup
                .unwrap_or(defaults.max_rollouts_per_startup)
                .clamp(
                    MIN_MEMORIES_MAX_ROLLOUTS_PER_STARTUP,
                    MAX_MEMORIES_MAX_ROLLOUTS_PER_STARTUP,
                ),
            min_rollout_idle_hours: toml
                .min_rollout_idle_hours
                .unwrap_or(defaults.min_rollout_idle_hours)
                .clamp(1, 48),
            min_rate_limit_remaining_percent: toml
                .min_rate_limit_remaining_percent
                .unwrap_or(defaults.min_rate_limit_remaining_percent)
                .clamp(0, 100),
            extract_model: toml.extract_model,
            consolidation_model: toml.consolidation_model,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Notifications {
    Enabled(bool),
    Custom(Vec<String>),
}

impl Default for Notifications {
    fn default() -> Self {
        Self::Enabled(true)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotificationMethod {
    #[default]
    Auto,
    Osc9,
    Bel,
}

impl fmt::Display for NotificationMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotificationMethod::Auto => write!(f, "auto"),
            NotificationMethod::Osc9 => write!(f, "osc9"),
            NotificationMethod::Bel => write!(f, "bel"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotificationCondition {
    /// Emit TUI notifications only while the terminal is unfocused.
    #[default]
    Unfocused,
    /// Emit TUI notifications regardless of terminal focus.
    Always,
}

impl fmt::Display for NotificationCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotificationCondition::Unfocused => write!(f, "unfocused"),
            NotificationCondition::Always => write!(f, "always"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TuiPetAnchor {
    /// Anchor the pet to the bottom of the current TUI composer viewport.
    #[default]
    Composer,
    /// Anchor the pet to the physical bottom of the terminal screen.
    ScreenBottom,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TuiNotificationSettings {
    /// Enable desktop notifications from the TUI.
    /// Defaults to `true`.
    #[serde(default, rename = "notifications")]
    pub notifications: Notifications,

    /// Notification method to use for terminal notifications.
    /// Defaults to `auto`.
    #[serde(default, rename = "notification_method")]
    pub method: NotificationMethod,

    /// Controls whether TUI notifications are delivered only when the terminal is unfocused or
    /// regardless of focus. Defaults to `unfocused`.
    #[serde(default, rename = "notification_condition")]
    pub condition: NotificationCondition,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModelAvailabilityNuxConfig {
    /// Number of times a startup availability NUX has been shown per model slug.
    #[serde(default, flatten)]
    pub shown_count: HashMap<String, u32>,
}

/// Fallback resize-reflow row cap when Sprite cannot identify a terminal-specific scrollback size.
pub const DEFAULT_TERMINAL_RESIZE_REFLOW_FALLBACK_MAX_ROWS: usize = 1_000;

/// Collection of settings that are specific to the TUI.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct Tui {
    #[serde(default, flatten)]
    pub notification_settings: TuiNotificationSettings,

    /// Enable animations (welcome screen, shimmer effects, spinners).
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub animations: bool,

    /// Show startup tooltips in the TUI welcome screen.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub show_tooltips: bool,

    /// Start the composer in Vim mode (`Normal`) by default.
    /// Defaults to `false`.
    #[serde(default)]
    pub vim_mode_default: bool,

    /// Start the TUI in raw scrollback mode for copy-friendly transcript output.
    /// Defaults to `false`.
    #[serde(default)]
    pub raw_output_mode: bool,

    /// Controls whether the TUI uses the terminal's alternate screen buffer.
    ///
    /// - `auto` (default): Use alternate screen.
    /// - `always`: Always use alternate screen.
    /// - `never`: Never use alternate screen (inline mode only, preserves scrollback).
    #[serde(default)]
    pub alternate_screen: AltScreenMode,

    /// Ordered list of status line item identifiers.
    ///
    /// When set, the TUI renders the selected items as the status line.
    /// When unset, the TUI defaults to: `model-with-reasoning` and `current-dir`.
    #[serde(default)]
    pub status_line: Option<Vec<String>>,

    /// Color status line items with colors derived from the active syntax theme.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub status_line_use_colors: bool,

    /// Ordered list of terminal title item identifiers.
    ///
    /// When set, the TUI renders the selected items into the terminal window/tab title.
    /// When unset, the TUI defaults to: `activity` and `project`.
    /// The `activity` item spins while working and shows an action-required
    /// message when blocked on the user.
    #[serde(default)]
    pub terminal_title: Option<Vec<String>>,

    /// Syntax highlighting theme name (kebab-case).
    ///
    /// When set, overrides automatic light/dark theme detection.
    /// Use `/theme` in the TUI or see `$SPRITE_HOME/themes` for custom themes.
    #[serde(default)]
    pub theme: Option<String>,

    /// Pet id to preselect in the terminal pet picker.
    ///
    /// Custom pet ids resolve against SPRITE_HOME/pets/<pet-id>/pet.json.
    #[serde(default)]
    pub pet: Option<String>,

    /// Where the terminal pet should anchor vertically.
    ///
    /// Defaults to `composer`, which follows the current TUI composer viewport.
    #[serde(default)]
    pub pet_anchor: TuiPetAnchor,

    /// Preferred layout for resume/fork session picker results.
    #[serde(default)]
    pub session_picker_view: Option<SessionPickerViewMode>,

    /// Keybinding overrides for the TUI.
    ///
    /// This supports rebinding selected actions globally and by context.
    /// Context bindings take precedence over `global` bindings.
    #[serde(default)]
    pub keymap: TuiKeymap,

    /// Startup tooltip availability NUX state persisted by the TUI.
    #[serde(default)]
    pub model_availability_nux: ModelAvailabilityNuxConfig,

    /// Trim terminal resize-reflow replay to the most recent rendered terminal rows when the
    /// transcript exceeds this cap. Omit to use Sprite's terminal-specific default. Set to `0` to
    /// keep all rendered rows.
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub terminal_resize_reflow_max_rows: Option<usize>,
}

const fn default_true() -> bool {
    true
}

pub use crate::skills_config::BundledSkillsConfig;
pub use crate::skills_config::SkillConfig;
pub use crate::skills_config::SkillsConfig;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SandboxWorkspaceWrite {
    #[serde(default)]
    pub writable_roots: Vec<AbsolutePathBuf>,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub exclude_tmpdir_env_var: bool,
    #[serde(default)]
    pub exclude_slash_tmp: bool,
}

impl From<SandboxWorkspaceWrite> for app_protocol::SandboxSettings {
    fn from(sandbox_workspace_write: SandboxWorkspaceWrite) -> Self {
        Self {
            writable_roots: sandbox_workspace_write.writable_roots,
            network_access: Some(sandbox_workspace_write.network_access),
            exclude_tmpdir_env_var: Some(sandbox_workspace_write.exclude_tmpdir_env_var),
            exclude_slash_tmp: Some(sandbox_workspace_write.exclude_slash_tmp),
        }
    }
}

/// Policy for building the `env` when spawning a process via shell-like tools.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ShellEnvironmentPolicyToml {
    pub inherit: Option<ShellEnvironmentPolicyInherit>,

    pub ignore_default_excludes: Option<bool>,

    /// List of regular expressions.
    pub exclude: Option<Vec<String>>,

    pub r#set: Option<HashMap<String, String>>,

    /// List of regular expressions.
    pub include_only: Option<Vec<String>>,

    pub experimental_use_profile: Option<bool>,
}

impl From<ShellEnvironmentPolicyToml> for ShellEnvironmentPolicy {
    fn from(toml: ShellEnvironmentPolicyToml) -> Self {
        // Default to inheriting the full environment when not specified.
        let inherit = toml.inherit.unwrap_or(ShellEnvironmentPolicyInherit::All);
        let ignore_default_excludes = toml.ignore_default_excludes.unwrap_or(true);
        let exclude = toml
            .exclude
            .unwrap_or_default()
            .into_iter()
            .map(|s| EnvironmentVariablePattern::new_case_insensitive(&s))
            .collect();
        let r#set = toml.r#set.unwrap_or_default();
        let include_only = toml
            .include_only
            .unwrap_or_default()
            .into_iter()
            .map(|s| EnvironmentVariablePattern::new_case_insensitive(&s))
            .collect();
        let use_profile = toml.experimental_use_profile.unwrap_or(false);

        Self {
            inherit,
            ignore_default_excludes,
            exclude,
            r#set,
            include_only,
            use_profile,
        }
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
