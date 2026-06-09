use shell_command::bash::extract_bash_command;
use shell_command::bash::parse_shell_lc_plain_commands;
use shell_command::powershell::extract_powershell_command;

const CANONICAL_BASH_SCRIPT_PREFIX: &str = "__sprite_shell_script__";
const CANONICAL_POWERSHELL_SCRIPT_PREFIX: &str = "__sprite_powershell_script__";

pub fn canonicalize_command_for_approval(command: &[String]) -> Vec<String> {
    if let Some(commands) = parse_shell_lc_plain_commands(command)
        && let [single_command] = commands.as_slice()
    {
        return single_command.clone();
    }

    if let Some((_shell, script)) = extract_bash_command(command) {
        let shell_mode = command.get(1).cloned().unwrap_or_default();
        return vec![
            CANONICAL_BASH_SCRIPT_PREFIX.to_string(),
            shell_mode,
            script.to_string(),
        ];
    }

    if let Some((_shell, script)) = extract_powershell_command(command) {
        return vec![
            CANONICAL_POWERSHELL_SCRIPT_PREFIX.to_string(),
            script.to_string(),
        ];
    }

    command.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_plain_bash_lc_command() {
        let command = vec![
            "bash".to_string(),
            "-lc".to_string(),
            "echo sprite".to_string(),
        ];

        assert_eq!(
            canonicalize_command_for_approval(&command),
            vec!["echo".to_string(), "sprite".to_string()]
        );
    }
}
