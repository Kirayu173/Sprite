use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::watch;

pub use crate::shell_snapshot::ShellSnapshot;
use crate::shell_snapshot::empty_shell_snapshot_receiver;
pub use shell_command::shell_detect::ShellType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shell {
    pub(crate) shell_type: ShellType,
    pub(crate) shell_path: PathBuf,
    #[serde(
        skip_serializing,
        skip_deserializing,
        default = "empty_shell_snapshot_receiver"
    )]
    pub(crate) shell_snapshot: watch::Receiver<Option<Arc<ShellSnapshot>>>,
}

impl Shell {
    pub fn name(&self) -> &'static str {
        self.shell_type.name()
    }

    pub fn derive_exec_args(&self, command: &str, use_login_shell: bool) -> Vec<String> {
        match self.shell_type {
            ShellType::Zsh | ShellType::Bash | ShellType::Sh => {
                let arg = if use_login_shell { "-lc" } else { "-c" };
                vec![
                    self.shell_path.to_string_lossy().to_string(),
                    arg.to_string(),
                    command.to_string(),
                ]
            }
            ShellType::PowerShell => {
                let mut args = vec![self.shell_path.to_string_lossy().to_string()];
                if !use_login_shell {
                    args.push("-NoProfile".to_string());
                }
                args.push("-Command".to_string());
                args.push(command.to_string());
                args
            }
            ShellType::Cmd => {
                vec![
                    self.shell_path.to_string_lossy().to_string(),
                    "/c".to_string(),
                    command.to_string(),
                ]
            }
        }
    }

    pub fn shell_snapshot(&self) -> Option<Arc<ShellSnapshot>> {
        self.shell_snapshot.borrow().clone()
    }
}

impl PartialEq for Shell {
    fn eq(&self, other: &Self) -> bool {
        self.shell_type == other.shell_type && self.shell_path == other.shell_path
    }
}

impl Eq for Shell {}

impl From<shell_command::shell_detect::DetectedShell> for Shell {
    fn from(detected: shell_command::shell_detect::DetectedShell) -> Self {
        Self {
            shell_type: detected.shell_type,
            shell_path: detected.shell_path,
            shell_snapshot: empty_shell_snapshot_receiver(),
        }
    }
}

pub fn get_shell_by_model_provided_path(shell_path: &PathBuf) -> Shell {
    shell_command::shell_detect::get_shell_by_model_provided_path(shell_path).into()
}

pub fn get_shell(shell_type: ShellType, path: Option<&PathBuf>) -> Option<Shell> {
    shell_command::shell_detect::get_shell(shell_type, path).map(Into::into)
}

pub fn default_user_shell() -> Shell {
    shell_command::shell_detect::default_user_shell().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_no_profile_when_not_login_shell() {
        let shell = Shell {
            shell_type: ShellType::PowerShell,
            shell_path: PathBuf::from("pwsh"),
            shell_snapshot: empty_shell_snapshot_receiver(),
        };

        assert_eq!(
            shell.derive_exec_args("Write-Output ok", false),
            vec!["pwsh", "-NoProfile", "-Command", "Write-Output ok"]
        );
    }

    #[test]
    fn bash_login_shell_uses_lc() {
        let shell = Shell {
            shell_type: ShellType::Bash,
            shell_path: PathBuf::from("bash"),
            shell_snapshot: empty_shell_snapshot_receiver(),
        };

        assert_eq!(
            shell.derive_exec_args("echo ok", true),
            vec!["bash", "-lc", "echo ok"]
        );
    }

    #[test]
    fn detected_shell_conversion_does_not_capture_snapshot() {
        let shell = Shell::from(shell_command::shell_detect::DetectedShell {
            shell_type: ShellType::Bash,
            shell_path: PathBuf::from("bash"),
        });

        assert!(shell.shell_snapshot().is_none());
    }
}
