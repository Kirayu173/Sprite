use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use exec_server::Environment;
use exec_server::ExecOutputStream;
use exec_server::ExecEnvPolicy;
use exec_server::ExecParams;
use exec_server::ExecProcess;
use exec_server::ExecServerRuntimePaths;
use exec_server::ProcessId;
use exec_server::TerminalSize;
use runtime_protocol::exec_output::ExecToolCallOutput;
use runtime_protocol::exec_output::StreamOutput;
use runtime_protocol::protocol::TruncationPolicy;
use tokio_util::sync::CancellationToken;
use utils_absolute_path::AbsolutePathBuf;

pub const DEFAULT_EXEC_COMMAND_TIMEOUT_MS: u64 = 10_000;
pub const EXEC_OUTPUT_MAX_BYTES: usize = utils_pty::DEFAULT_OUTPUT_BYTES_CAP;
const EXEC_READ_WAIT_MS: u64 = 100;
const EXEC_TIMEOUT_EXIT_CODE: i32 = 124;
const EXEC_CANCELLED_EXIT_CODE: i32 = 1;

#[derive(Debug, Clone)]
pub struct LocalExecParams {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub env: HashMap<String, String>,
    pub env_policy: Option<ExecEnvPolicy>,
    pub terminal_size: TerminalSize,
    pub expiration: ExecExpiration,
    pub arg0: Option<String>,
    pub runtime_paths: ExecServerRuntimePaths,
}

#[derive(Clone, Debug)]
pub enum ExecExpiration {
    Timeout(Duration),
    DefaultTimeout,
    Cancellation(CancellationToken),
    TimeoutOrCancellation {
        timeout: Duration,
        cancellation: CancellationToken,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecExpirationOutcome {
    TimedOut,
    Cancelled,
}

impl From<Option<u64>> for ExecExpiration {
    fn from(timeout_ms: Option<u64>) -> Self {
        timeout_ms.map_or(ExecExpiration::DefaultTimeout, |timeout_ms| {
            ExecExpiration::Timeout(Duration::from_millis(timeout_ms))
        })
    }
}

impl From<u64> for ExecExpiration {
    fn from(timeout_ms: u64) -> Self {
        ExecExpiration::Timeout(Duration::from_millis(timeout_ms))
    }
}

impl ExecExpiration {
    pub async fn wait_with_outcome(self) -> ExecExpirationOutcome {
        match self {
            ExecExpiration::Timeout(duration) => {
                tokio::time::sleep(duration).await;
                ExecExpirationOutcome::TimedOut
            }
            ExecExpiration::DefaultTimeout => {
                tokio::time::sleep(Duration::from_millis(DEFAULT_EXEC_COMMAND_TIMEOUT_MS)).await;
                ExecExpirationOutcome::TimedOut
            }
            ExecExpiration::Cancellation(cancel) => {
                cancel.cancelled().await;
                ExecExpirationOutcome::Cancelled
            }
            ExecExpiration::TimeoutOrCancellation {
                timeout,
                cancellation,
            } => {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => ExecExpirationOutcome::Cancelled,
                    _ = tokio::time::sleep(timeout) => ExecExpirationOutcome::TimedOut,
                }
            }
        }
    }
}

pub async fn execute_local_command(
    params: LocalExecParams,
) -> Result<ExecToolCallOutput, exec_server::ExecServerError> {
    let environment = Environment::local(params.runtime_paths);
    let backend = environment.get_exec_backend();
    let process = backend
        .start(ExecParams {
            process_id: ProcessId::from(format!("runtime-exec-{}", uuid::Uuid::new_v4())),
            argv: params.command,
            cwd: params.cwd.into_path_buf(),
            env_policy: params.env_policy,
            env: params.env,
            tty: false,
            terminal_size: params.terminal_size,
            pipe_stdin: false,
            arg0: params.arg0,
        })
        .await?
        .process;

    collect_process_output(
        process,
        ProcessOutputCollection {
            expiration: Some(params.expiration),
            max_bytes: EXEC_OUTPUT_MAX_BYTES,
            truncation_policy: TruncationPolicy::Bytes(EXEC_OUTPUT_MAX_BYTES),
        },
    )
    .await
}

pub(crate) struct ProcessOutputCollection {
    pub(crate) expiration: Option<ExecExpiration>,
    pub(crate) max_bytes: usize,
    pub(crate) truncation_policy: TruncationPolicy,
}

pub(crate) struct ProcessOutputSnapshot {
    pub(crate) stdout: StreamOutput<String>,
    pub(crate) stderr: StreamOutput<String>,
    pub(crate) aggregated_output: StreamOutput<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration: Duration,
    pub(crate) timed_out: bool,
    pub(crate) cancelled: bool,
}

pub(crate) async fn collect_process_snapshot(
    process: Arc<dyn ExecProcess>,
    collection: ProcessOutputCollection,
) -> Result<ProcessOutputSnapshot, exec_server::ExecServerError> {
    let start = Instant::now();
    let mut after_seq = None;
    let mut stdout = HeadTailBuffer::new(collection.max_bytes);
    let mut stderr = HeadTailBuffer::new(collection.max_bytes);
    let mut exit_code = None;
    let mut timed_out = false;
    let mut cancelled = false;

    let expiration_wait = async {
        match collection.expiration {
            Some(expiration) => Some(expiration.wait_with_outcome().await),
            None => std::future::pending::<Option<ExecExpirationOutcome>>().await,
        }
    };
    tokio::pin!(expiration_wait);

    loop {
        let read = process.read(
            after_seq,
            Some(collection.max_bytes),
            Some(EXEC_READ_WAIT_MS),
        );
        tokio::select! {
            response = read => {
                let response = response?;
                for chunk in &response.chunks {
                    match chunk.stream {
                        ExecOutputStream::Stdout | ExecOutputStream::Pty => {
                            stdout.push(chunk.chunk.as_slice());
                        }
                        ExecOutputStream::Stderr => {
                            stderr.push(chunk.chunk.as_slice());
                        }
                    }
                }
                after_seq = response.next_seq.checked_sub(1).or(after_seq);
                exit_code = response.exit_code.or(exit_code);
                if response.closed {
                    break;
                }
            }
            outcome = &mut expiration_wait => {
                match outcome {
                    Some(ExecExpirationOutcome::TimedOut) => {
                        timed_out = true;
                        exit_code = Some(EXEC_TIMEOUT_EXIT_CODE);
                    }
                    Some(ExecExpirationOutcome::Cancelled) => {
                        cancelled = true;
                        exit_code = Some(EXEC_CANCELLED_EXIT_CODE);
                    }
                    None => unreachable!("expiration wait only resolves when configured"),
                }
                let _ = process.terminate().await;
                drain_process_output(
                    Arc::clone(&process),
                    after_seq,
                    &mut stdout,
                    &mut stderr,
                    collection.max_bytes,
                    &mut exit_code,
                ).await?;
                break;
            }
        }
    }

    let stdout = output_stream(stdout, collection.truncation_policy);
    let stderr = output_stream(stderr, collection.truncation_policy);
    let aggregated_output = aggregate_streams(&stdout, &stderr, collection.truncation_policy);

    Ok(ProcessOutputSnapshot {
        stdout,
        stderr,
        aggregated_output,
        exit_code,
        duration: start.elapsed(),
        timed_out,
        cancelled,
    })
}

pub(crate) async fn collect_process_output(
    process: Arc<dyn ExecProcess>,
    collection: ProcessOutputCollection,
) -> Result<ExecToolCallOutput, exec_server::ExecServerError> {
    let snapshot = collect_process_snapshot(process, collection).await?;
    Ok(ExecToolCallOutput {
        exit_code: snapshot.exit_code.unwrap_or(-1),
        stdout: snapshot.stdout,
        stderr: snapshot.stderr,
        aggregated_output: snapshot.aggregated_output,
        duration: snapshot.duration,
        timed_out: snapshot.timed_out,
    })
}

async fn drain_process_output(
    process: Arc<dyn ExecProcess>,
    mut after_seq: Option<u64>,
    stdout: &mut HeadTailBuffer,
    stderr: &mut HeadTailBuffer,
    max_bytes: usize,
    exit_code: &mut Option<i32>,
) -> Result<(), exec_server::ExecServerError> {
    for _ in 0..20 {
        let response = process.read(after_seq, Some(max_bytes), Some(10)).await?;
        for chunk in &response.chunks {
            match chunk.stream {
                ExecOutputStream::Stdout | ExecOutputStream::Pty => {
                    stdout.push(chunk.chunk.as_slice())
                }
                ExecOutputStream::Stderr => stderr.push(chunk.chunk.as_slice()),
            }
        }
        after_seq = response.next_seq.checked_sub(1).or(after_seq);
        if exit_code.is_none() {
            *exit_code = response.exit_code;
        }
        if response.closed || response.chunks.is_empty() {
            break;
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct HeadTailBuffer {
    cap: usize,
    head_cap: usize,
    tail_cap: usize,
    head: Vec<u8>,
    tail: Vec<u8>,
    total: usize,
}

impl HeadTailBuffer {
    pub(crate) fn new(cap: usize) -> Self {
        let head_cap = cap / 2;
        let tail_cap = cap.saturating_sub(head_cap);
        Self {
            cap,
            head_cap,
            tail_cap,
            head: Vec::with_capacity(cap),
            tail: Vec::new(),
            total: 0,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        if self.cap == 0 {
            return;
        }

        if self.total <= self.cap {
            self.head.extend_from_slice(bytes);
            return;
        }

        if self.head.len() > self.head_cap {
            let overflow = self.head.split_off(self.head_cap);
            self.tail.extend_from_slice(&overflow);
        }

        let remaining_head = self.head_cap.saturating_sub(self.head.len());
        let (tail_input, head_input) = if remaining_head > 0 {
            let split = bytes.len().min(remaining_head);
            (&bytes[split..], &bytes[..split])
        } else {
            (bytes, &[][..])
        };
        self.head.extend_from_slice(head_input);
        self.tail.extend_from_slice(tail_input);
        if self.tail.len() > self.tail_cap {
            let drain = self.tail.len() - self.tail_cap;
            self.tail.drain(..drain);
        }
    }

    pub(crate) fn into_text(self) -> (String, Option<u32>) {
        let omitted = self.total.saturating_sub(self.head.len() + self.tail.len());
        let mut bytes = self.head;
        if omitted > 0 {
            bytes.extend_from_slice(format!("\n... {omitted} bytes truncated ...\n").as_bytes());
            bytes.extend_from_slice(&self.tail);
        }
        let text = runtime_protocol::exec_output::bytes_to_string_smart(&bytes);
        let truncated_after_lines = if omitted > 0 {
            Some(text.lines().count().try_into().unwrap_or(u32::MAX))
        } else {
            None
        };
        (text, truncated_after_lines)
    }
}

fn output_stream(buffer: HeadTailBuffer, policy: TruncationPolicy) -> StreamOutput<String> {
    let (text, truncated_after_lines) = buffer.into_text();
    let text = utils_output_truncation::truncate_text(&text, policy);
    StreamOutput {
        text,
        truncated_after_lines,
    }
}

fn aggregate_streams(
    stdout: &StreamOutput<String>,
    stderr: &StreamOutput<String>,
    policy: TruncationPolicy,
) -> StreamOutput<String> {
    let text = if stderr.text.is_empty() {
        stdout.text.clone()
    } else if stdout.text.is_empty() {
        stderr.text.clone()
    } else {
        format!("{}{}", stdout.text, stderr.text)
    };

    StreamOutput {
        text: utils_output_truncation::truncate_text(&text, policy),
        truncated_after_lines: stdout
            .truncated_after_lines
            .or(stderr.truncated_after_lines),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_paths() -> ExecServerRuntimePaths {
        ExecServerRuntimePaths::new(std::env::current_exe().unwrap(), None).unwrap()
    }

    fn current_dir() -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap()).unwrap()
    }

    fn shell_command(script: &str) -> Vec<String> {
        if cfg!(windows) {
            vec!["cmd".to_string(), "/c".to_string(), script.to_string()]
        } else {
            vec!["sh".to_string(), "-c".to_string(), script.to_string()]
        }
    }

    async fn execute(script: &str, expiration: ExecExpiration) -> ExecToolCallOutput {
        execute_local_command(LocalExecParams {
            command: shell_command(script),
            cwd: current_dir(),
            env: HashMap::new(),
            env_policy: None,
            terminal_size: TerminalSize::default(),
            expiration,
            arg0: None,
            runtime_paths: runtime_paths(),
        })
        .await
        .expect("exec should run")
    }

    #[tokio::test]
    async fn runs_local_command() {
        let output = execute(
            if cfg!(windows) {
                "echo sprite-exec-ok"
            } else {
                "echo sprite-exec-ok"
            },
            ExecExpiration::Timeout(Duration::from_secs(5)),
        )
        .await;

        assert!(
            output.aggregated_output.text.contains("sprite-exec-ok"),
            "stdout={:?} stderr={:?}",
            output.stdout.text,
            output.stderr.text
        );
        assert_eq!(output.exit_code, 0);
        assert!(output.duration > Duration::ZERO);
    }

    #[tokio::test]
    async fn preserves_nonzero_exit_code() {
        let output = execute(
            if cfg!(windows) { "exit /b 7" } else { "exit 7" },
            ExecExpiration::Timeout(Duration::from_secs(5)),
        )
        .await;

        assert_eq!(output.exit_code, 7);
        assert!(!output.timed_out);
    }

    #[tokio::test]
    async fn timeout_is_total_deadline_not_per_read() {
        let script = if cfg!(windows) {
            "echo tick & for /l %i in (1,1,1000000000) do @rem"
        } else {
            "for i in $(seq 1 20); do echo tick; sleep 0.1; done"
        };
        let start = Instant::now();
        let output = execute(script, ExecExpiration::Timeout(Duration::from_millis(250))).await;

        assert_eq!(output.exit_code, EXEC_TIMEOUT_EXIT_CODE);
        assert!(output.timed_out);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "timeout should not reset while output arrives"
        );
    }

    #[tokio::test]
    async fn cancellation_terminates_process_without_marking_timeout() {
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        });

        let output = execute(
            if cfg!(windows) {
                "ping -n 10 127.0.0.1 >nul"
            } else {
                "sleep 5"
            },
            ExecExpiration::Cancellation(cancellation),
        )
        .await;

        assert_eq!(output.exit_code, EXEC_CANCELLED_EXIT_CODE);
        assert!(!output.timed_out);
    }

    #[tokio::test]
    async fn truncates_output_with_head_and_tail_marker() {
        let mut buffer = HeadTailBuffer::new(12);
        buffer.push(b"abcdef");
        buffer.push(b"ghijklmnop");
        let (text, truncated_after_lines) = buffer.into_text();

        assert!(text.contains("abcdef"));
        assert!(text.contains("truncated"));
        assert!(text.contains("mnop"));
        assert!(truncated_after_lines.is_some());
    }
}
