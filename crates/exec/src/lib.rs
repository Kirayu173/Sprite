use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use exec_server::Environment;
use exec_server::ExecEnvPolicy;
use exec_server::ExecOutputStream;
use exec_server::ExecParams;
use exec_server::ExecServerRuntimePaths;
use exec_server::ProcessId;
use exec_server::ReadResponse;
use exec_server::TerminalSize;

#[derive(Debug, Clone)]
pub struct SpriteExecRequest {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub env_policy: Option<ExecEnvPolicy>,
    pub tty: bool,
    pub terminal_size: TerminalSize,
    pub pipe_stdin: bool,
    pub arg0: Option<String>,
    pub runtime_paths: ExecServerRuntimePaths,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteExecOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub async fn run_local_command(
    request: SpriteExecRequest,
) -> Result<SpriteExecOutput, exec_server::ExecServerError> {
    let environment = Environment::local(request.runtime_paths);
    let backend = environment.get_exec_backend();
    let process = backend
        .start(ExecParams {
            process_id: ProcessId::from(format!("sprite-exec-{}", uuid::Uuid::new_v4())),
            argv: request.argv,
            cwd: request.cwd,
            env_policy: request.env_policy,
            env: request.env,
            tty: request.tty,
            terminal_size: request.terminal_size,
            pipe_stdin: request.pipe_stdin,
            arg0: request.arg0,
        })
        .await?
        .process;

    collect_process_output(process).await
}

async fn collect_process_output(
    process: Arc<dyn exec_server::ExecProcess>,
) -> Result<SpriteExecOutput, exec_server::ExecServerError> {
    let mut after_seq = None;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    loop {
        let response = process.read(after_seq, None, Some(100)).await?;
        append_chunks(&response, &mut stdout, &mut stderr);
        after_seq = response.next_seq.checked_sub(1).or(after_seq);

        if response.closed {
            return Ok(SpriteExecOutput {
                exit_code: response.exit_code.unwrap_or(-1),
                stdout,
                stderr,
            });
        }
    }
}

fn append_chunks(response: &ReadResponse, stdout: &mut Vec<u8>, stderr: &mut Vec<u8>) {
    for chunk in &response.chunks {
        match chunk.stream {
            ExecOutputStream::Stdout | ExecOutputStream::Pty => {
                stdout.extend_from_slice(chunk.chunk.as_slice());
            }
            ExecOutputStream::Stderr => stderr.extend_from_slice(chunk.chunk.as_slice()),
        }
    }
}
