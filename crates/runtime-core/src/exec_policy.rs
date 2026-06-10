use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use config::{ConfigLayerSource, ConfigLayerStack, ConfigLayerStackOrdering, RuntimeConfig};
use execpolicy::{
    AmendError, Decision, Error as ExecPolicyRuleError, Evaluation, MatchOptions,
    NetworkRuleProtocol, Policy, PolicyParser, RuleMatch, blocking_append_allow_prefix_rule,
    blocking_append_network_rule,
};
use runtime_protocol::approvals::ExecPolicyAmendment;
use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::{PermissionProfile, SandboxPermissions};
use runtime_protocol::permissions::FileSystemSandboxKind;
use runtime_protocol::protocol::AskForApproval;
use shell_command::bash::{parse_shell_lc_plain_commands, parse_shell_lc_single_command_prefix};
use shell_command::is_dangerous_command::command_might_be_dangerous;
#[cfg(windows)]
use shell_command::is_dangerous_command::is_dangerous_powershell_words;
use shell_command::is_safe_command::is_known_safe_command;
#[cfg(windows)]
use shell_command::is_safe_command::is_safe_powershell_words;
#[cfg(windows)]
use shell_command::powershell::parse_powershell_command_into_plain_commands;
use tokio::{fs, sync::Semaphore, task::spawn_blocking};
use tracing::instrument;
use utils_absolute_path::AbsolutePathBuf;

use crate::tools::sandboxing::ExecApprovalRequirement;

const PROMPT_CONFLICT_REASON: &str =
    "approval required by policy, but AskForApproval is set to Never";
const REJECT_SANDBOX_APPROVAL_REASON: &str =
    "approval required by policy, but granular sandbox approval is disabled";
const REJECT_RULES_APPROVAL_REASON: &str =
    "approval required by policy rule, but granular rules approval is disabled";
const RULES_DIR_NAME: &str = "rules";
const RULE_EXTENSION: &str = "rules";
const DEFAULT_POLICY_FILE: &str = "default.rules";
static BANNED_PREFIX_SUGGESTIONS: &[&[&str]] = &[
    &["python3"],
    &["python3", "-"],
    &["python3", "-c"],
    &["python"],
    &["python", "-"],
    &["python", "-c"],
    &["py"],
    &["py", "-3"],
    &["pythonw"],
    &["pyw"],
    &["pypy"],
    &["pypy3"],
    &["git"],
    &["bash"],
    &["bash", "-lc"],
    &["sh"],
    &["sh", "-c"],
    &["sh", "-lc"],
    &["zsh"],
    &["zsh", "-lc"],
    &["/bin/zsh"],
    &["/bin/zsh", "-lc"],
    &["/bin/bash"],
    &["/bin/bash", "-lc"],
    &["pwsh"],
    &["pwsh", "-Command"],
    &["pwsh", "-c"],
    &["powershell"],
    &["powershell", "-Command"],
    &["powershell", "-c"],
    &["powershell.exe"],
    &["powershell.exe", "-Command"],
    &["powershell.exe", "-c"],
    &["env"],
    &["sudo"],
    &["node"],
    &["node", "-e"],
    &["perl"],
    &["perl", "-e"],
    &["ruby"],
    &["ruby", "-e"],
    &["php"],
    &["php", "-r"],
    &["lua"],
    &["lua", "-e"],
    &["osascript"],
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecPolicyCommandOrigin {
    Generic,
    #[cfg(windows)]
    PowerShell,
}

#[derive(Clone, Copy)]
pub struct UnmatchedCommandContext<'a> {
    pub approval_policy: AskForApproval,
    pub permission_profile: &'a PermissionProfile,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub sandbox_permissions: SandboxPermissions,
    pub used_complex_parsing: bool,
    pub command_origin: ExecPolicyCommandOrigin,
}

#[derive(Debug, Eq, PartialEq)]
struct ExecPolicyCommands {
    commands: Vec<Vec<String>>,
    used_complex_parsing: bool,
    command_origin: ExecPolicyCommandOrigin,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecPolicyError {
    #[error("failed to read rules files from {dir}: {source}")]
    ReadDir {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read rules file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse rules file {path}: {source}")]
    ParsePolicy {
        path: String,
        source: execpolicy::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ExecPolicyUpdateError {
    #[error("failed to update rules file {path}: {source}")]
    AppendRule { path: PathBuf, source: AmendError },
    #[error("failed to join blocking rules update task: {source}")]
    JoinBlockingTask { source: tokio::task::JoinError },
    #[error("failed to update in-memory rules: {source}")]
    AddRule {
        #[from]
        source: ExecPolicyRuleError,
    },
}

pub struct ExecPolicyManager {
    policy: ArcSwap<Policy>,
    update_lock: Semaphore,
}

pub struct ExecApprovalRequest<'a> {
    pub command: &'a [String],
    pub approval_policy: AskForApproval,
    pub permission_profile: PermissionProfile,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub sandbox_permissions: SandboxPermissions,
    pub prefix_rule: Option<Vec<String>>,
}

impl Default for ExecPolicyManager {
    fn default() -> Self {
        Self::new(Arc::new(Policy::empty()))
    }
}

impl ExecPolicyManager {
    pub fn new(policy: Arc<Policy>) -> Self {
        Self {
            policy: ArcSwap::from(policy),
            update_lock: Semaphore::new(1),
        }
    }

    #[instrument(level = "info", skip_all)]
    pub async fn load(config_stack: &ConfigLayerStack) -> Result<Self, ExecPolicyError> {
        let (policy, warning) = load_exec_policy_with_warning(config_stack)?;
        if let Some(err) = warning.as_ref() {
            tracing::warn!("{}", format_exec_policy_error_with_source(err));
        }
        Ok(Self::new(Arc::new(policy)))
    }

    pub fn current(&self) -> Arc<Policy> {
        self.policy.load_full()
    }

    pub async fn create_exec_approval_requirement_for_command(
        &self,
        req: ExecApprovalRequest<'_>,
    ) -> ExecApprovalRequirement {
        let ExecApprovalRequest {
            command,
            approval_policy,
            permission_profile,
            windows_sandbox_level,
            sandbox_permissions,
            prefix_rule,
        } = req;
        let exec_policy = self.current();
        let ExecPolicyCommands {
            commands,
            used_complex_parsing,
            command_origin,
        } = commands_for_exec_policy(command);
        let auto_amendment_allowed = !used_complex_parsing;
        let exec_policy_fallback = |cmd: &[String]| {
            render_decision_for_unmatched_command(
                cmd,
                UnmatchedCommandContext {
                    approval_policy,
                    permission_profile: &permission_profile,
                    windows_sandbox_level,
                    sandbox_permissions,
                    used_complex_parsing,
                    command_origin,
                },
            )
        };
        let match_options = MatchOptions {
            resolve_host_executables: true,
        };
        let evaluation = exec_policy.check_multiple_with_options(
            commands.iter(),
            &exec_policy_fallback,
            &match_options,
        );

        let requested_amendment = if auto_amendment_allowed {
            derive_requested_execpolicy_amendment_from_prefix_rule(
                prefix_rule.as_ref(),
                &evaluation.matched_rules,
                exec_policy.as_ref(),
                &commands,
                &exec_policy_fallback,
                &match_options,
            )
        } else {
            None
        };

        match evaluation.decision {
            Decision::Forbidden => ExecApprovalRequirement::Forbidden {
                reason: derive_forbidden_reason(command, &evaluation),
            },
            Decision::Prompt => {
                let prompt_is_rule = evaluation.matched_rules.iter().any(|rule_match| {
                    is_policy_match(rule_match) && rule_match.decision() == Decision::Prompt
                });
                match prompt_is_rejected_by_policy(approval_policy, prompt_is_rule) {
                    Some(reason) => ExecApprovalRequirement::Forbidden {
                        reason: reason.to_string(),
                    },
                    None => ExecApprovalRequirement::NeedsApproval {
                        reason: derive_prompt_reason(command, &evaluation),
                        proposed_execpolicy_amendment: requested_amendment.or_else(|| {
                            if auto_amendment_allowed {
                                try_derive_execpolicy_amendment_for_prompt_rules(
                                    &evaluation.matched_rules,
                                )
                            } else {
                                None
                            }
                        }),
                    },
                }
            }
            Decision::Allow => ExecApprovalRequirement::Skip {
                bypass_sandbox: commands.iter().all(|command| {
                    exec_policy
                        .matches_for_command_with_options(command, None, &match_options)
                        .iter()
                        .any(|rule_match| {
                            is_policy_match(rule_match) && rule_match.decision() == Decision::Allow
                        })
                }),
                proposed_execpolicy_amendment: if auto_amendment_allowed {
                    try_derive_execpolicy_amendment_for_allow_rules(&evaluation.matched_rules)
                } else {
                    None
                },
            },
        }
    }

    pub async fn append_amendment_and_update(
        &self,
        sprite_home: &Path,
        amendment: &ExecPolicyAmendment,
    ) -> Result<(), ExecPolicyUpdateError> {
        let _update_guard =
            self.update_lock
                .acquire()
                .await
                .map_err(|_| ExecPolicyUpdateError::AddRule {
                    source: ExecPolicyRuleError::InvalidRule(
                        "exec policy update semaphore closed".to_string(),
                    ),
                })?;
        let policy_path = default_policy_path(sprite_home);
        spawn_blocking({
            let policy_path = policy_path.clone();
            let prefix = amendment.command.clone();
            move || blocking_append_allow_prefix_rule(&policy_path, &prefix)
        })
        .await
        .map_err(|source| ExecPolicyUpdateError::JoinBlockingTask { source })?
        .map_err(|source| ExecPolicyUpdateError::AppendRule {
            path: policy_path,
            source,
        })?;

        let mut updated_policy = self.current().as_ref().clone();
        updated_policy.add_prefix_rule(&amendment.command, Decision::Allow)?;
        self.policy.store(Arc::new(updated_policy));
        Ok(())
    }

    pub async fn append_network_rule_and_update(
        &self,
        sprite_home: &Path,
        host: &str,
        protocol: NetworkRuleProtocol,
        decision: Decision,
        justification: Option<String>,
    ) -> Result<(), ExecPolicyUpdateError> {
        let _update_guard =
            self.update_lock
                .acquire()
                .await
                .map_err(|_| ExecPolicyUpdateError::AddRule {
                    source: ExecPolicyRuleError::InvalidRule(
                        "exec policy update semaphore closed".to_string(),
                    ),
                })?;
        let policy_path = default_policy_path(sprite_home);
        let host = host.to_string();
        spawn_blocking({
            let policy_path = policy_path.clone();
            let host = host.clone();
            let justification = justification.clone();
            move || {
                blocking_append_network_rule(
                    &policy_path,
                    &host,
                    protocol,
                    decision,
                    justification.as_deref(),
                )
            }
        })
        .await
        .map_err(|source| ExecPolicyUpdateError::JoinBlockingTask { source })?
        .map_err(|source| ExecPolicyUpdateError::AppendRule {
            path: policy_path,
            source,
        })?;

        let mut updated_policy = self.current().as_ref().clone();
        updated_policy.add_network_rule(&host, protocol, decision, justification)?;
        self.policy.store(Arc::new(updated_policy));
        Ok(())
    }
}

pub fn child_uses_parent_exec_policy(
    parent_config: &RuntimeConfig,
    child_config: &RuntimeConfig,
) -> bool {
    fn exec_policy_config_folders(config: &RuntimeConfig) -> Vec<AbsolutePathBuf> {
        config
            .config_layer_stack
            .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, false)
            .into_iter()
            .filter_map(config::ConfigLayerEntry::config_folder)
            .collect()
    }

    exec_policy_config_folders(parent_config) == exec_policy_config_folders(child_config)
        && parent_config
            .config_layer_stack
            .ignore_user_and_project_exec_policy_rules()
            == child_config
                .config_layer_stack
                .ignore_user_and_project_exec_policy_rules()
        && parent_config.config_layer_stack.requirements().exec_policy
            == child_config.config_layer_stack.requirements().exec_policy
}

pub fn prompt_is_rejected_by_policy(
    approval_policy: AskForApproval,
    prompt_is_rule: bool,
) -> Option<&'static str> {
    match approval_policy {
        AskForApproval::Never => Some(PROMPT_CONFLICT_REASON),
        AskForApproval::OnFailure | AskForApproval::OnRequest | AskForApproval::UnlessTrusted => {
            None
        }
        AskForApproval::Granular(granular) => {
            if prompt_is_rule {
                (!granular.allows_rules_approval()).then_some(REJECT_RULES_APPROVAL_REASON)
            } else {
                (!granular.allows_sandbox_approval()).then_some(REJECT_SANDBOX_APPROVAL_REASON)
            }
        }
    }
}

pub async fn check_execpolicy_for_warnings(
    config_stack: &ConfigLayerStack,
) -> Result<Option<ExecPolicyError>, ExecPolicyError> {
    let (_, warning) = load_exec_policy_with_warning(config_stack)?;
    Ok(warning)
}

pub fn format_exec_policy_error_with_source(error: &ExecPolicyError) -> String {
    match error {
        ExecPolicyError::ParsePolicy { path, source } => {
            let rendered_source = source.to_string();
            let structured_location = source
                .location()
                .map(|location| (PathBuf::from(location.path), location.range.start.line));
            let parsed_location = parse_starlark_line_from_message(&rendered_source);
            let location = match (structured_location, parsed_location) {
                (Some((_, 1)), Some((parsed_path, parsed_line))) if parsed_line > 1 => {
                    Some((parsed_path, parsed_line))
                }
                (Some(structured), _) => Some(structured),
                (None, parsed) => parsed,
            };
            let message = exec_policy_message_for_display(source);
            match location {
                Some((path, line)) => {
                    format!(
                        "{}:{}: {} (problem is on or around line {})",
                        path.display(),
                        line,
                        message,
                        line
                    )
                }
                None => format!("{path}: {message}"),
            }
        }
        _ => error.to_string(),
    }
}

pub fn load_exec_policy_sync(config_stack: &ConfigLayerStack) -> Result<Policy, ExecPolicyError> {
    futures::executor::block_on(load_exec_policy(config_stack))
}

pub async fn load_exec_policy(config_stack: &ConfigLayerStack) -> Result<Policy, ExecPolicyError> {
    let mut policy_paths = Vec::new();
    for layer in config_stack.get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, false) {
        if config_stack.ignore_user_and_project_exec_policy_rules()
            && matches!(
                layer.name,
                ConfigLayerSource::User { .. } | ConfigLayerSource::Project { .. }
            )
        {
            continue;
        }
        if let Some(config_folder) = layer.config_folder() {
            let policy_dir = config_folder.join(RULES_DIR_NAME);
            policy_paths.extend(collect_policy_files(&policy_dir).await?);
        }
    }

    let mut parser = PolicyParser::new();
    for policy_path in &policy_paths {
        let contents =
            fs::read_to_string(policy_path)
                .await
                .map_err(|source| ExecPolicyError::ReadFile {
                    path: policy_path.clone(),
                    source,
                })?;
        let identifier = policy_path.to_string_lossy().to_string();
        parser
            .parse(&identifier, &contents)
            .map_err(|source| ExecPolicyError::ParsePolicy {
                path: identifier,
                source,
            })?;
    }

    let policy = parser.build();
    let Some(requirements_policy) = config_stack.requirements().exec_policy.as_deref() else {
        return Ok(policy);
    };

    Ok(policy.merge_overlay(requirements_policy.as_ref()))
}

pub fn render_decision_for_unmatched_command(
    command: &[String],
    context: UnmatchedCommandContext<'_>,
) -> Decision {
    let UnmatchedCommandContext {
        approval_policy,
        permission_profile,
        windows_sandbox_level,
        sandbox_permissions,
        used_complex_parsing,
        command_origin,
    } = context;
    let file_system_sandbox_policy = permission_profile.file_system_sandbox_policy();
    let is_known_safe = match command_origin {
        ExecPolicyCommandOrigin::Generic => is_known_safe_command(command),
        #[cfg(windows)]
        ExecPolicyCommandOrigin::PowerShell => is_safe_powershell_words(command),
    };
    let windows_managed_fs_restrictions_without_sandbox_backend = cfg!(windows)
        && windows_sandbox_level == WindowsSandboxLevel::Disabled
        && profile_has_managed_filesystem_restrictions(permission_profile);

    if is_known_safe
        && !used_complex_parsing
        && (approval_policy == AskForApproval::UnlessTrusted
            || windows_managed_fs_restrictions_without_sandbox_backend)
    {
        return Decision::Allow;
    }

    let command_is_dangerous = match command_origin {
        ExecPolicyCommandOrigin::Generic => command_might_be_dangerous(command),
        #[cfg(windows)]
        ExecPolicyCommandOrigin::PowerShell => is_dangerous_powershell_words(command),
    };
    if command_is_dangerous || windows_managed_fs_restrictions_without_sandbox_backend {
        return match approval_policy {
            AskForApproval::Never => {
                if matches!(
                    permission_profile,
                    PermissionProfile::Disabled | PermissionProfile::External { .. }
                ) {
                    Decision::Allow
                } else {
                    Decision::Forbidden
                }
            }
            AskForApproval::OnFailure
            | AskForApproval::OnRequest
            | AskForApproval::UnlessTrusted
            | AskForApproval::Granular(_) => Decision::Prompt,
        };
    }

    match approval_policy {
        AskForApproval::Never | AskForApproval::OnFailure => Decision::Allow,
        AskForApproval::UnlessTrusted => Decision::Prompt,
        AskForApproval::OnRequest | AskForApproval::Granular(_) => {
            match file_system_sandbox_policy.kind {
                FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                    Decision::Allow
                }
                FileSystemSandboxKind::Restricted => {
                    if sandbox_permissions.requests_sandbox_override() {
                        Decision::Prompt
                    } else {
                        Decision::Allow
                    }
                }
            }
        }
    }
}

fn load_exec_policy_with_warning(
    config_stack: &ConfigLayerStack,
) -> Result<(Policy, Option<ExecPolicyError>), ExecPolicyError> {
    futures::executor::block_on(async {
        match load_exec_policy(config_stack).await {
            Ok(policy) => Ok((policy, None)),
            Err(err @ ExecPolicyError::ParsePolicy { .. }) => Ok((Policy::empty(), Some(err))),
            Err(err) => Err(err),
        }
    })
}

async fn collect_policy_files(dir: &Path) -> Result<Vec<PathBuf>, ExecPolicyError> {
    let mut read_dir = match fs::read_dir(dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ExecPolicyError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            });
        }
    };

    let mut policy_paths = Vec::new();
    while let Some(entry) =
        read_dir
            .next_entry()
            .await
            .map_err(|source| ExecPolicyError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            })?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| ExecPolicyError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            })?;

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == RULE_EXTENSION)
            && file_type.is_file()
        {
            policy_paths.push(path);
        }
    }

    policy_paths.sort();
    Ok(policy_paths)
}

fn commands_for_exec_policy(command: &[String]) -> ExecPolicyCommands {
    if let Some(commands) = parse_shell_lc_plain_commands(command)
        && !commands.is_empty()
    {
        return ExecPolicyCommands {
            commands,
            used_complex_parsing: false,
            command_origin: ExecPolicyCommandOrigin::Generic,
        };
    }

    #[cfg(windows)]
    if let Some(commands) = parse_powershell_command_into_plain_commands(command)
        && !commands.is_empty()
    {
        return ExecPolicyCommands {
            commands,
            used_complex_parsing: false,
            command_origin: ExecPolicyCommandOrigin::PowerShell,
        };
    }

    if let Some(single_command) = parse_shell_lc_single_command_prefix(command) {
        return ExecPolicyCommands {
            commands: vec![single_command],
            used_complex_parsing: true,
            command_origin: ExecPolicyCommandOrigin::Generic,
        };
    }

    ExecPolicyCommands {
        commands: vec![command.to_vec()],
        used_complex_parsing: false,
        command_origin: ExecPolicyCommandOrigin::Generic,
    }
}

fn is_policy_match(rule_match: &RuleMatch) -> bool {
    matches!(rule_match, RuleMatch::PrefixRuleMatch { .. })
}

fn profile_has_managed_filesystem_restrictions(permission_profile: &PermissionProfile) -> bool {
    let file_system_sandbox_policy = permission_profile.file_system_sandbox_policy();
    matches!(permission_profile, PermissionProfile::Managed { .. })
        && matches!(
            file_system_sandbox_policy.kind,
            FileSystemSandboxKind::Restricted
        )
        && !file_system_sandbox_policy.has_full_disk_write_access()
}

fn default_policy_path(sprite_home: &Path) -> PathBuf {
    sprite_home.join(RULES_DIR_NAME).join(DEFAULT_POLICY_FILE)
}

fn try_derive_execpolicy_amendment_for_prompt_rules(
    matched_rules: &[RuleMatch],
) -> Option<ExecPolicyAmendment> {
    if matched_rules
        .iter()
        .any(|rule_match| is_policy_match(rule_match) && rule_match.decision() == Decision::Prompt)
    {
        return None;
    }

    matched_rules
        .iter()
        .find_map(|rule_match| match rule_match {
            RuleMatch::HeuristicsRuleMatch {
                command,
                decision: Decision::Prompt,
            } => Some(ExecPolicyAmendment::from(command.clone())),
            _ => None,
        })
}

fn try_derive_execpolicy_amendment_for_allow_rules(
    matched_rules: &[RuleMatch],
) -> Option<ExecPolicyAmendment> {
    if matched_rules.iter().any(is_policy_match) {
        return None;
    }

    matched_rules
        .iter()
        .find_map(|rule_match| match rule_match {
            RuleMatch::HeuristicsRuleMatch {
                command,
                decision: Decision::Allow,
            } => Some(ExecPolicyAmendment::from(command.clone())),
            _ => None,
        })
}

fn derive_requested_execpolicy_amendment_from_prefix_rule(
    prefix_rule: Option<&Vec<String>>,
    matched_rules: &[RuleMatch],
    exec_policy: &Policy,
    commands: &[Vec<String>],
    exec_policy_fallback: &impl Fn(&[String]) -> Decision,
    match_options: &MatchOptions,
) -> Option<ExecPolicyAmendment> {
    let prefix_rule = prefix_rule?;
    if prefix_rule.is_empty() {
        return None;
    }
    if BANNED_PREFIX_SUGGESTIONS.iter().any(|banned| {
        prefix_rule.len() == banned.len()
            && prefix_rule
                .iter()
                .map(String::as_str)
                .eq(banned.iter().copied())
    }) {
        return None;
    }
    if matched_rules.iter().any(is_policy_match) {
        return None;
    }

    let amendment = ExecPolicyAmendment::new(prefix_rule.clone());
    if prefix_rule_would_approve_all_commands(
        exec_policy,
        &amendment.command,
        commands,
        exec_policy_fallback,
        match_options,
    ) {
        Some(amendment)
    } else {
        None
    }
}

fn prefix_rule_would_approve_all_commands(
    exec_policy: &Policy,
    prefix_rule: &[String],
    commands: &[Vec<String>],
    exec_policy_fallback: &impl Fn(&[String]) -> Decision,
    match_options: &MatchOptions,
) -> bool {
    let mut policy_with_prefix_rule = exec_policy.clone();
    if policy_with_prefix_rule
        .add_prefix_rule(prefix_rule, Decision::Allow)
        .is_err()
    {
        return false;
    }

    commands.iter().all(|command| {
        policy_with_prefix_rule
            .check_with_options(command, exec_policy_fallback, match_options)
            .decision
            == Decision::Allow
    })
}

fn derive_prompt_reason(command_args: &[String], evaluation: &Evaluation) -> Option<String> {
    let command = command_args.join(" ");
    let most_specific_prompt = evaluation
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Prompt,
                justification,
                ..
            } => Some((matched_prefix.len(), justification.as_deref())),
            _ => None,
        })
        .max_by_key(|(matched_prefix_len, _)| *matched_prefix_len);

    match most_specific_prompt {
        Some((_matched_prefix_len, Some(justification))) => {
            Some(format!("`{command}` requires approval: {justification}"))
        }
        Some((_matched_prefix_len, None)) => {
            Some(format!("`{command}` requires approval by policy"))
        }
        None => None,
    }
}

fn derive_forbidden_reason(command_args: &[String], evaluation: &Evaluation) -> String {
    let command = command_args.join(" ");

    let most_specific_forbidden = evaluation
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Forbidden,
                justification,
                ..
            } => Some((matched_prefix, justification.as_deref())),
            _ => None,
        })
        .max_by_key(|(matched_prefix, _)| matched_prefix.len());

    match most_specific_forbidden {
        Some((_matched_prefix, Some(justification))) => {
            format!("`{command}` rejected: {justification}")
        }
        Some((matched_prefix, None)) => {
            format!(
                "`{command}` rejected: policy forbids commands starting with `{}`",
                matched_prefix.join(" ")
            )
        }
        None => format!("`{command}` rejected: blocked by policy"),
    }
}

fn exec_policy_message_for_display(source: &execpolicy::Error) -> String {
    let message = source.to_string();
    if let Some(line) = message
        .lines()
        .find(|line| line.trim_start().starts_with("error: "))
    {
        return line.to_owned();
    }
    if let Some(first_line) = message.lines().next()
        && let Some((_, detail)) = first_line.rsplit_once(": starlark error: ")
    {
        return detail.trim().to_string();
    }
    message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parse_starlark_line_from_message(message: &str) -> Option<(PathBuf, usize)> {
    let first_line = message.lines().next()?.trim();
    let (path_and_position, _) = first_line.rsplit_once(": starlark error:")?;
    let mut parts = path_and_position.rsplitn(3, ':');
    let _column = parts.next()?.parse::<usize>().ok()?;
    let line = parts.next()?.parse::<usize>().ok()?;
    let path = PathBuf::from(parts.next()?);
    (line > 0).then_some((path, line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::RuntimeConfigBuilder;
    use runtime_protocol::permissions::{
        FileSystemAccessMode, FileSystemPath, FileSystemSandboxEntry,
    };

    #[test]
    fn prompt_rejection_respects_granular_flags() {
        let policy = AskForApproval::Granular(runtime_protocol::protocol::GranularApprovalConfig {
            rules: false,
            sandbox_approval: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        });
        assert_eq!(
            prompt_is_rejected_by_policy(policy, true),
            Some(REJECT_RULES_APPROVAL_REASON)
        );
    }

    #[test]
    fn unmatched_safe_command_is_allowed_for_unless_trusted() {
        let profile = PermissionProfile::read_only();
        let decision = render_decision_for_unmatched_command(
            &["ls".to_string()],
            UnmatchedCommandContext {
                approval_policy: AskForApproval::UnlessTrusted,
                permission_profile: &profile,
                windows_sandbox_level: WindowsSandboxLevel::Disabled,
                sandbox_permissions: SandboxPermissions::UseDefault,
                used_complex_parsing: false,
                command_origin: ExecPolicyCommandOrigin::Generic,
            },
        );
        assert_eq!(decision, Decision::Allow);
    }

    #[test]
    fn unmatched_dangerous_command_prompts_when_approval_available() {
        let profile = PermissionProfile::read_only();
        let decision = render_decision_for_unmatched_command(
            &["rm".to_string(), "-rf".to_string(), "/".to_string()],
            UnmatchedCommandContext {
                approval_policy: AskForApproval::OnRequest,
                permission_profile: &profile,
                windows_sandbox_level: WindowsSandboxLevel::Disabled,
                sandbox_permissions: SandboxPermissions::UseDefault,
                used_complex_parsing: false,
                command_origin: ExecPolicyCommandOrigin::Generic,
            },
        );
        assert_eq!(decision, Decision::Prompt);
    }

    #[tokio::test]
    async fn collect_policy_files_ignores_missing_directory() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("rules");
        let files = collect_policy_files(&missing).await.unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn commands_for_exec_policy_parses_bash_lc() {
        let parsed = commands_for_exec_policy(&[
            "bash".to_string(),
            "-lc".to_string(),
            "echo ok && pwd".to_string(),
        ]);
        assert_eq!(parsed.commands.len(), 2);
        assert_eq!(parsed.command_origin, ExecPolicyCommandOrigin::Generic);
    }

    #[cfg(windows)]
    #[test]
    fn commands_for_exec_policy_parses_powershell_command() {
        let parsed = commands_for_exec_policy(&[
            "powershell.exe".to_string(),
            "-Command".to_string(),
            "Write-Host ok; Get-Location".to_string(),
        ]);
        assert_eq!(parsed.commands.len(), 2);
        assert_eq!(parsed.command_origin, ExecPolicyCommandOrigin::PowerShell);
    }

    #[cfg(windows)]
    #[test]
    fn unmatched_safe_powershell_command_is_allowed_for_unless_trusted() {
        let profile = PermissionProfile::read_only();
        let decision = render_decision_for_unmatched_command(
            &["Get-ChildItem".to_string()],
            UnmatchedCommandContext {
                approval_policy: AskForApproval::UnlessTrusted,
                permission_profile: &profile,
                windows_sandbox_level: WindowsSandboxLevel::Disabled,
                sandbox_permissions: SandboxPermissions::UseDefault,
                used_complex_parsing: false,
                command_origin: ExecPolicyCommandOrigin::PowerShell,
            },
        );
        assert_eq!(decision, Decision::Allow);
    }

    #[cfg(windows)]
    #[test]
    fn unmatched_dangerous_powershell_command_prompts_when_approval_available() {
        let profile = PermissionProfile::read_only();
        let decision = render_decision_for_unmatched_command(
            &[
                "Remove-Item".to_string(),
                "-Recurse".to_string(),
                "C:\\".to_string(),
            ],
            UnmatchedCommandContext {
                approval_policy: AskForApproval::OnRequest,
                permission_profile: &profile,
                windows_sandbox_level: WindowsSandboxLevel::Disabled,
                sandbox_permissions: SandboxPermissions::UseDefault,
                used_complex_parsing: false,
                command_origin: ExecPolicyCommandOrigin::PowerShell,
            },
        );
        assert_eq!(decision, Decision::Prompt);
    }

    #[test]
    fn derived_forbidden_reason_uses_justification() {
        let mut policy = Policy::empty();
        policy
            .add_prefix_rule(&["rm".to_string()], Decision::Forbidden)
            .unwrap();
        let evaluation = policy.check(&["rm".to_string()], &|_| Decision::Prompt);
        assert!(
            derive_forbidden_reason(&["rm".to_string()], &evaluation).contains("policy forbids")
        );
    }

    #[tokio::test]
    async fn child_execpolicy_match_uses_layer_stack_shape() {
        let cwd = AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap()).unwrap();
        let first = RuntimeConfigBuilder::default()
            .cwd(cwd.clone())
            .load()
            .await
            .unwrap();
        let second = RuntimeConfigBuilder::default()
            .cwd(cwd)
            .load()
            .await
            .unwrap();
        assert!(child_uses_parent_exec_policy(&first, &second));
    }

    #[test]
    fn managed_fs_restrictions_detects_writable_limits() {
        let profile = PermissionProfile::Managed {
            file_system: runtime_protocol::models::ManagedFileSystemPermissions::Restricted {
                entries: vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: AbsolutePathBuf::from_absolute_path(std::env::temp_dir()).unwrap(),
                    },
                    access: FileSystemAccessMode::Read,
                }],
                glob_scan_max_depth: None,
            },
            network: runtime_protocol::permissions::NetworkSandboxPolicy::Restricted,
        };
        assert!(profile_has_managed_filesystem_restrictions(&profile));
    }
}
