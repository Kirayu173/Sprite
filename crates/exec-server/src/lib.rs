mod environment;
mod local_process;
mod process;
mod process_id;
mod protocol;
mod runtime_paths;

pub use environment::Environment;
pub use environment::EnvironmentManager;
pub use environment::LOCAL_ENVIRONMENT_ID;
pub use local_process::LocalProcess;
pub use process::ExecBackend;
pub use process::ExecProcess;
pub use process::ExecProcessEvent;
pub use process::ExecProcessEventReceiver;
pub use process::StartedExecProcess;
pub use process_id::ProcessId;
pub use protocol::ByteChunk;
pub use protocol::EnvironmentInfo;
pub use protocol::ExecEnvPolicy;
pub use protocol::ExecOutputStream;
pub use protocol::ExecParams;
pub use protocol::ExecResponse;
pub use protocol::ProcessOutputChunk;
pub use protocol::ReadParams;
pub use protocol::ReadResponse;
pub use protocol::ResizeParams;
pub use protocol::ResizeResponse;
pub use protocol::ShellInfo;
pub use protocol::TerminalSize;
pub use protocol::TerminateParams;
pub use protocol::TerminateResponse;
pub use protocol::WriteParams;
pub use protocol::WriteResponse;
pub use protocol::WriteStatus;
pub use runtime_paths::ExecServerRuntimePaths;

#[derive(Debug, thiserror::Error)]
pub enum ExecServerError {
    #[error("failed to spawn process: {0}")]
    Spawn(String),
    #[error("exec-server protocol error: {0}")]
    Protocol(String),
    #[error("exec-server rejected request ({code}): {message}")]
    Server { code: i64, message: String },
}
