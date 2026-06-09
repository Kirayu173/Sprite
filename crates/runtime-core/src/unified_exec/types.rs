use std::collections::HashMap;

use exec_server::ExecEnvPolicy;
use exec_server::ExecServerRuntimePaths;
use exec_server::TerminalSize;
use runtime_protocol::exec_output::StreamOutput;
use runtime_protocol::protocol::TruncationPolicy;
use utils_absolute_path::AbsolutePathBuf;

pub const MIN_YIELD_TIME_MS: u64 = 250;
pub const MIN_EMPTY_YIELD_TIME_MS: u64 = 5_000;
pub const MAX_YIELD_TIME_MS: u64 = 30_000;
pub const DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
pub const UNIFIED_EXEC_OUTPUT_MAX_BYTES: usize = 1024 * 1024;
pub const UNIFIED_EXEC_OUTPUT_MAX_TOKENS: usize = UNIFIED_EXEC_OUTPUT_MAX_BYTES / 4;
pub const MAX_UNIFIED_EXEC_PROCESSES: usize = 64;

#[derive(Debug, Clone)]
pub struct ExecCommandRequest {
    pub command: Vec<String>,
    pub process_id: i32,
    pub yield_time_ms: u64,
    pub max_output_tokens: Option<usize>,
    pub cwd: AbsolutePathBuf,
    pub env: HashMap<String, String>,
    pub env_policy: Option<ExecEnvPolicy>,
    pub tty: bool,
    pub terminal_size: TerminalSize,
    pub arg0: Option<String>,
    pub runtime_paths: ExecServerRuntimePaths,
}

#[derive(Debug, Clone)]
pub struct WriteStdinRequest {
    pub process_id: i32,
    pub input: String,
    pub yield_time_ms: u64,
    pub max_output_tokens: Option<usize>,
    pub truncation_policy: TruncationPolicy,
}

#[derive(Debug, Clone)]
pub struct UnifiedExecResponse {
    pub process_id: i32,
    pub stdout: StreamOutput<String>,
    pub stderr: StreamOutput<String>,
    pub aggregated_output: StreamOutput<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum UnifiedExecError {
    #[error(transparent)]
    ExecServer(#[from] exec_server::ExecServerError),
    #[error("process {0} does not exist")]
    UnknownProcess(i32),
    #[error("process {0} stdin is closed")]
    StdinClosed(i32),
    #[error("process {0} is still starting")]
    ProcessStarting(i32),
    #[error("process {0} already exists")]
    DuplicateProcess(i32),
    #[error("maximum unified exec process count reached")]
    ProcessLimitReached,
}

pub fn clamp_yield_time(yield_time_ms: u64) -> u64 {
    yield_time_ms.clamp(MIN_YIELD_TIME_MS, MAX_YIELD_TIME_MS)
}

pub fn resolve_max_tokens(max_tokens: Option<usize>) -> usize {
    max_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

pub fn truncation_policy(max_tokens: Option<usize>) -> TruncationPolicy {
    TruncationPolicy::Tokens(resolve_max_tokens(max_tokens))
}
