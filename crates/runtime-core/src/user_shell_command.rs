use std::collections::HashMap;

use exec_server::ExecEnvPolicy;
use exec_server::ExecServerRuntimePaths;
use exec_server::TerminalSize;
use runtime_protocol::config_types::ShellEnvironmentPolicy;
use utils_absolute_path::AbsolutePathBuf;

use crate::exec::ExecExpiration;
use crate::exec::LocalExecParams;
use crate::shell::Shell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserShellCommand {
    pub command: String,
}

impl UserShellCommand {
    pub fn new(command: impl Into<String>) -> Result<Self, UserShellCommandError> {
        let command = command.into();
        if command.trim().is_empty() {
            return Err(UserShellCommandError::EmptyCommand);
        }
        Ok(Self { command })
    }

    pub fn history_text(&self) -> String {
        format!("!{}", self.command)
    }

    pub fn transcript_text(&self, exit_code: i32, stdout: &str, stderr: &str) -> String {
        let mut text = format!("$ {}\n[exit_code: {exit_code}]", self.command);
        if !stdout.is_empty() {
            text.push_str("\n[stdout]\n");
            text.push_str(stdout);
        }
        if !stderr.is_empty() {
            text.push_str("\n[stderr]\n");
            text.push_str(stderr);
        }
        text
    }

    pub fn into_exec_params(
        self,
        shell: &Shell,
        cwd: AbsolutePathBuf,
        env: HashMap<String, String>,
        env_policy: Option<ExecEnvPolicy>,
        expiration: ExecExpiration,
        runtime_paths: ExecServerRuntimePaths,
    ) -> LocalExecParams {
        LocalExecParams {
            command: shell.derive_exec_args(&self.command, false),
            cwd,
            env,
            env_policy,
            terminal_size: TerminalSize::default(),
            expiration,
            arg0: None,
            runtime_paths,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserShellCommandError {
    #[error("user shell command must not be empty")]
    EmptyCommand,
}

pub fn exec_env_policy_from_shell_policy(policy: &ShellEnvironmentPolicy) -> ExecEnvPolicy {
    ExecEnvPolicy {
        inherit: policy.inherit.clone(),
        ignore_default_excludes: policy.ignore_default_excludes,
        exclude: policy.exclude.iter().map(ToString::to_string).collect(),
        r#set: policy.r#set.clone(),
        include_only: policy
            .include_only
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::shell::ShellType;

    #[test]
    fn rejects_empty_command() {
        assert!(matches!(
            UserShellCommand::new("  "),
            Err(UserShellCommandError::EmptyCommand)
        ));
    }

    #[test]
    fn formats_history_and_transcript() {
        let command = UserShellCommand::new("echo ok").expect("command");

        assert_eq!(command.history_text(), "!echo ok");
        assert_eq!(
            command.transcript_text(0, "ok\n", ""),
            "$ echo ok\n[exit_code: 0]\n[stdout]\nok\n"
        );
    }

    #[test]
    fn builds_shell_exec_args() {
        let command = UserShellCommand::new("Write-Output ok").expect("command");
        let shell = crate::shell::get_shell(ShellType::PowerShell, Some(&PathBuf::from("pwsh")))
            .expect("shell");
        let params = command.into_exec_params(
            &shell,
            AbsolutePathBuf::from_absolute_path(std::env::current_dir().expect("cwd"))
                .expect("abs cwd"),
            HashMap::new(),
            None,
            ExecExpiration::DefaultTimeout,
            ExecServerRuntimePaths::new(std::env::current_exe().expect("exe"), None)
                .expect("runtime paths"),
        );

        assert!(
            params.command[0]
                .to_ascii_lowercase()
                .contains("powershell")
        );
        assert_eq!(
            &params.command[1..],
            ["-NoProfile", "-Command", "Write-Output ok"]
        );
    }
}
