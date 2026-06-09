use std::sync::Arc;
use std::time::Duration;

use exec_server::Environment;
use exec_server::ExecParams;
use exec_server::ProcessId;
use exec_server::WriteStatus;
use runtime_protocol::protocol::TruncationPolicy;
use tokio::sync::Mutex;

use crate::exec::ProcessOutputCollection;
use crate::exec::collect_process_snapshot;
use crate::unified_exec::store::ProcessStore;
use crate::unified_exec::types::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use crate::unified_exec::types::MAX_YIELD_TIME_MS;
use crate::unified_exec::types::MIN_EMPTY_YIELD_TIME_MS;
use crate::unified_exec::types::MIN_YIELD_TIME_MS;
use crate::unified_exec::types::UNIFIED_EXEC_OUTPUT_MAX_BYTES;
use crate::unified_exec::types::ExecCommandRequest;
use crate::unified_exec::types::UnifiedExecError;
use crate::unified_exec::types::UnifiedExecResponse;
use crate::unified_exec::types::WriteStdinRequest;
use crate::unified_exec::types::clamp_yield_time;
use crate::unified_exec::types::truncation_policy;

pub struct UnifiedExecProcessManager {
    process_store: Mutex<ProcessStore>,
    max_write_stdin_yield_time_ms: u64,
}

impl UnifiedExecProcessManager {
    pub fn new(max_write_stdin_yield_time_ms: u64) -> Self {
        Self {
            process_store: Mutex::new(ProcessStore::default()),
            max_write_stdin_yield_time_ms: max_write_stdin_yield_time_ms
                .max(MIN_EMPTY_YIELD_TIME_MS),
        }
    }

    pub async fn exec_command(
        &self,
        request: ExecCommandRequest,
    ) -> Result<UnifiedExecResponse, UnifiedExecError> {
        let process_id = request.process_id;
        let environment = Environment::local(request.runtime_paths);
        let backend = environment.get_exec_backend();

        {
            let store = self.process_store.lock().await;
            store.reserve_process_id(process_id)?;
        }

        let process = backend
            .start(ExecParams {
                process_id: ProcessId::from(format!("unified-exec-{process_id}")),
                argv: request.command,
                cwd: request.cwd.into_path_buf(),
                env_policy: request.env_policy,
                env: request.env,
                tty: request.tty,
                terminal_size: request.terminal_size,
                pipe_stdin: true,
                arg0: request.arg0,
            })
            .await?
            .process;

        {
            let mut store = self.process_store.lock().await;
            store.insert_process(process_id, Arc::clone(&process));
        }

        self.read_process(
            process_id,
            clamp_yield_time(request.yield_time_ms),
            truncation_policy(request.max_output_tokens),
        )
        .await
    }

    pub async fn write_stdin(
        &self,
        request: WriteStdinRequest,
    ) -> Result<UnifiedExecResponse, UnifiedExecError> {
        let process = {
            let store = self.process_store.lock().await;
            store.process(request.process_id)?
        };

        match process.write(request.input.into_bytes()).await?.status {
            WriteStatus::Accepted => {}
            WriteStatus::UnknownProcess => {
                return Err(UnifiedExecError::UnknownProcess(request.process_id));
            }
            WriteStatus::StdinClosed => {
                return Err(UnifiedExecError::StdinClosed(request.process_id));
            }
            WriteStatus::Starting => {
                return Err(UnifiedExecError::ProcessStarting(request.process_id));
            }
        }

        let yield_time_ms = if request.yield_time_ms == 0 {
            MIN_EMPTY_YIELD_TIME_MS
        } else {
            request
                .yield_time_ms
                .min(self.max_write_stdin_yield_time_ms)
                .clamp(MIN_YIELD_TIME_MS, MAX_YIELD_TIME_MS)
        };
        self.read_process(request.process_id, yield_time_ms, request.truncation_policy)
            .await
    }

    pub async fn terminate_process(&self, process_id: i32) -> Result<(), UnifiedExecError> {
        let entry = {
            let mut store = self.process_store.lock().await;
            store.remove_process(process_id)?
        };
        entry.process.terminate().await?;
        Ok(())
    }

    async fn read_process(
        &self,
        process_id: i32,
        yield_time_ms: u64,
        truncation_policy: TruncationPolicy,
    ) -> Result<UnifiedExecResponse, UnifiedExecError> {
        let (process, after_seq) = {
            let store = self.process_store.lock().await;
            store.process_with_after_seq(process_id)?
        };

        let snapshot = collect_process_snapshot(
            process,
            ProcessOutputCollection {
                expiration: None,
                max_bytes: UNIFIED_EXEC_OUTPUT_MAX_BYTES,
                truncation_policy,
            },
        );
        let read_until_yield = tokio::time::timeout(Duration::from_millis(yield_time_ms), snapshot);

        let snapshot = match read_until_yield.await {
            Ok(result) => result?,
            Err(_) => return self.read_until_yield(process_id, after_seq, truncation_policy).await,
        };

        let mut store = self.process_store.lock().await;
        store.update_exit_and_remove_if_done(process_id, snapshot.exit_code);

        Ok(UnifiedExecResponse {
            process_id,
            stdout: snapshot.stdout,
            stderr: snapshot.stderr,
            aggregated_output: snapshot.aggregated_output,
            exit_code: snapshot.exit_code,
            timed_out: snapshot.timed_out || snapshot.cancelled,
        })
    }

    async fn read_until_yield(
        &self,
        process_id: i32,
        after_seq: Option<u64>,
        truncation_policy: TruncationPolicy,
    ) -> Result<UnifiedExecResponse, UnifiedExecError> {
        let process = {
            let store = self.process_store.lock().await;
            store.process(process_id)?
        };
        let response = process.read(after_seq, None, Some(0)).await?;

        let mut store = self.process_store.lock().await;
        store.append_output(process_id, &response)?;
        store.response(process_id, truncation_policy)
    }
}

impl Default for UnifiedExecProcessManager {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS)
    }
}
