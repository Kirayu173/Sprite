use std::path::PathBuf;

use crate::shell_command_support::shell_detect::ShellType;
use crate::shell_command_support::shell_detect::detect_shell_type;

const POWERSHELL_FLAGS: &[&str] = &["-nologo", "-noprofile", "-command", "-c"];

pub fn extract_powershell_command(command: &[String]) -> Option<(&str, &str)> {
    if command.len() < 3 {
        return None;
    }

    let shell = &command[0];
    if !matches!(
        detect_shell_type(PathBuf::from(shell)),
        Some(ShellType::PowerShell)
    ) {
        return None;
    }

    let mut i = 1usize;
    while i + 1 < command.len() {
        let flag = command[i].to_ascii_lowercase();
        if !POWERSHELL_FLAGS.contains(&flag.as_str()) {
            return None;
        }
        if matches!(flag.as_str(), "-command" | "-c") {
            return Some((shell.as_str(), command[i + 1].as_str()));
        }
        i += 1;
    }

    None
}
