use std::collections::HashMap;

use runtime_protocol::ThreadId;
use runtime_protocol::config_types::ShellEnvironmentPolicy;
use runtime_protocol::shell_environment;

pub use runtime_protocol::shell_environment::SPRITE_THREAD_ID_ENV_VAR;

pub fn create_env(
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<ThreadId>,
) -> HashMap<String, String> {
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    shell_environment::create_env(policy, thread_id.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::config_types::ShellEnvironmentPolicyInherit;

    #[test]
    fn policy_none_excludes_host_environment() {
        let env = create_env(
            &ShellEnvironmentPolicy {
                inherit: ShellEnvironmentPolicyInherit::None,
                ignore_default_excludes: true,
                exclude: Vec::new(),
                r#set: HashMap::from([("SPRITE_ENV".to_string(), "ok".to_string())]),
                include_only: Vec::new(),
                use_profile: false,
            },
            None,
        );

        assert_eq!(env.get("SPRITE_ENV"), Some(&"ok".to_string()));
    }
}
