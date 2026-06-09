use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use crate::ExecBackend;
use crate::ExecServerError;
use crate::ExecServerRuntimePaths;
use crate::LocalProcess;
use crate::protocol::EnvironmentInfo;
use crate::protocol::ShellInfo;

pub const LOCAL_ENVIRONMENT_ID: &str = "local";

#[derive(Debug)]
pub struct EnvironmentManager {
    environments: RwLock<HashMap<String, Arc<Environment>>>,
}

impl EnvironmentManager {
    pub fn default_for_tests() -> Self {
        Self::local_only()
    }

    pub fn local_only() -> Self {
        Self {
            environments: RwLock::new(HashMap::from([(
                LOCAL_ENVIRONMENT_ID.to_string(),
                Arc::new(Environment::default_for_tests()),
            )])),
        }
    }

    pub fn default_environment(&self) -> Option<Arc<Environment>> {
        self.get_environment(LOCAL_ENVIRONMENT_ID)
    }

    pub fn default_environment_id(&self) -> Option<&str> {
        Some(LOCAL_ENVIRONMENT_ID)
    }

    pub fn default_environment_ids(&self) -> Vec<String> {
        vec![LOCAL_ENVIRONMENT_ID.to_string()]
    }

    pub fn try_local_environment(&self) -> Option<Arc<Environment>> {
        self.default_environment()
    }

    pub fn default_or_local_environment(&self) -> Option<Arc<Environment>> {
        self.default_environment()
    }

    pub fn get_environment(&self, environment_id: &str) -> Option<Arc<Environment>> {
        self.environments
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(environment_id)
            .cloned()
    }
}

#[derive(Clone)]
pub struct Environment {
    exec_backend: Arc<dyn ExecBackend>,
    local_runtime_paths: Option<ExecServerRuntimePaths>,
}

impl Environment {
    pub fn default_for_tests() -> Self {
        Self {
            exec_backend: Arc::new(LocalProcess::default()),
            local_runtime_paths: None,
        }
    }

    pub fn local(local_runtime_paths: ExecServerRuntimePaths) -> Self {
        Self {
            exec_backend: Arc::new(LocalProcess::default()),
            local_runtime_paths: Some(local_runtime_paths),
        }
    }

    pub fn create(
        exec_server_url: Option<String>,
        local_runtime_paths: ExecServerRuntimePaths,
    ) -> Result<Self, ExecServerError> {
        if let Some(exec_server_url) = exec_server_url.filter(|url| !url.trim().is_empty()) {
            return Err(ExecServerError::Protocol(format!(
                "remote exec environments are not supported in Sprite: {exec_server_url}"
            )));
        }
        Ok(Self::local(local_runtime_paths))
    }

    pub fn create_for_tests(exec_server_url: Option<String>) -> Result<Self, ExecServerError> {
        if let Some(exec_server_url) = exec_server_url.filter(|url| !url.trim().is_empty()) {
            return Err(ExecServerError::Protocol(format!(
                "remote exec environments are not supported in Sprite: {exec_server_url}"
            )));
        }
        Ok(Self::default_for_tests())
    }

    pub fn is_remote(&self) -> bool {
        false
    }

    pub fn exec_server_url(&self) -> Option<&str> {
        None
    }

    pub fn local_runtime_paths(&self) -> Option<&ExecServerRuntimePaths> {
        self.local_runtime_paths.as_ref()
    }

    pub async fn info(&self) -> Result<EnvironmentInfo, ExecServerError> {
        Ok(EnvironmentInfo::local())
    }

    pub fn get_exec_backend(&self) -> Arc<dyn ExecBackend> {
        Arc::clone(&self.exec_backend)
    }
}

impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Environment")
            .field("remote", &false)
            .finish()
    }
}

impl EnvironmentInfo {
    pub(crate) fn local() -> Self {
        Self {
            shell: shell_command::shell_detect::default_user_shell().into(),
        }
    }
}

impl From<shell_command::shell_detect::DetectedShell> for ShellInfo {
    fn from(shell: shell_command::shell_detect::DetectedShell) -> Self {
        Self {
            name: shell.name().to_string(),
            path: shell.shell_path.to_string_lossy().into_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_rejects_remote_exec_server_url() {
        let err = Environment::create_for_tests(Some("https://example.invalid".to_string()))
            .expect_err("remote exec should be explicit unsupported");
        assert!(err.to_string().contains("remote exec environments"));
    }

    #[test]
    fn create_accepts_absent_remote_url() {
        let environment =
            Environment::create_for_tests(None).expect("local test environment should create");
        assert!(!environment.is_remote());
    }
}
