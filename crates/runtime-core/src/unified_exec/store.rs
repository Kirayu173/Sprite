use std::collections::HashMap;
use std::sync::Arc;

use exec_server::ExecOutputStream;
use exec_server::ExecProcess;
use runtime_protocol::protocol::TruncationPolicy;

use crate::exec::HeadTailBuffer;
use crate::unified_exec::output::aggregate_streams;
use crate::unified_exec::output::output_stream;
use crate::unified_exec::types::MAX_UNIFIED_EXEC_PROCESSES;
use crate::unified_exec::types::UNIFIED_EXEC_OUTPUT_MAX_BYTES;
use crate::unified_exec::types::UnifiedExecError;
use crate::unified_exec::types::UnifiedExecResponse;

#[derive(Default)]
pub(super) struct ProcessStore {
    processes: HashMap<i32, ProcessEntry>,
}

pub(super) struct ProcessEntry {
    pub(super) process: Arc<dyn ExecProcess>,
    stdout: HeadTailBuffer,
    stderr: HeadTailBuffer,
    pub(super) after_seq: Option<u64>,
    exit_code: Option<i32>,
}

impl ProcessStore {
    pub(super) fn reserve_process_id(&self, process_id: i32) -> Result<(), UnifiedExecError> {
        if self.processes.contains_key(&process_id) {
            return Err(UnifiedExecError::DuplicateProcess(process_id));
        }
        if self.processes.len() >= MAX_UNIFIED_EXEC_PROCESSES {
            return Err(UnifiedExecError::ProcessLimitReached);
        }
        Ok(())
    }

    pub(super) fn insert_process(&mut self, process_id: i32, process: Arc<dyn ExecProcess>) {
        self.processes
            .insert(process_id, ProcessEntry::new(process));
    }

    pub(super) fn remove_process(
        &mut self,
        process_id: i32,
    ) -> Result<ProcessEntry, UnifiedExecError> {
        self.processes
            .remove(&process_id)
            .ok_or(UnifiedExecError::UnknownProcess(process_id))
    }

    pub(super) fn process(
        &self,
        process_id: i32,
    ) -> Result<Arc<dyn ExecProcess>, UnifiedExecError> {
        self.processes
            .get(&process_id)
            .map(|entry| Arc::clone(&entry.process))
            .ok_or(UnifiedExecError::UnknownProcess(process_id))
    }

    pub(super) fn process_with_after_seq(
        &self,
        process_id: i32,
    ) -> Result<(Arc<dyn ExecProcess>, Option<u64>), UnifiedExecError> {
        let entry = self
            .processes
            .get(&process_id)
            .ok_or(UnifiedExecError::UnknownProcess(process_id))?;
        Ok((Arc::clone(&entry.process), entry.after_seq))
    }

    pub(super) fn append_output(
        &mut self,
        process_id: i32,
        response: &exec_server::ReadResponse,
    ) -> Result<(), UnifiedExecError> {
        let entry = self
            .processes
            .get_mut(&process_id)
            .ok_or(UnifiedExecError::UnknownProcess(process_id))?;
        entry.append_output(response);
        Ok(())
    }

    pub(super) fn update_exit_and_remove_if_done(
        &mut self,
        process_id: i32,
        exit_code: Option<i32>,
    ) {
        if let Some(entry) = self.processes.get_mut(&process_id) {
            entry.exit_code = exit_code.or(entry.exit_code);
            if exit_code.is_some() {
                self.processes.remove(&process_id);
            }
        }
    }

    pub(super) fn response(
        &self,
        process_id: i32,
        truncation_policy: TruncationPolicy,
    ) -> Result<UnifiedExecResponse, UnifiedExecError> {
        self.processes
            .get(&process_id)
            .ok_or(UnifiedExecError::UnknownProcess(process_id))
            .map(|entry| entry.response(process_id, truncation_policy))
    }
}

impl ProcessEntry {
    fn new(process: Arc<dyn ExecProcess>) -> Self {
        Self {
            process,
            stdout: HeadTailBuffer::new(UNIFIED_EXEC_OUTPUT_MAX_BYTES),
            stderr: HeadTailBuffer::new(UNIFIED_EXEC_OUTPUT_MAX_BYTES),
            after_seq: None,
            exit_code: None,
        }
    }

    fn append_output(&mut self, response: &exec_server::ReadResponse) {
        for chunk in &response.chunks {
            match chunk.stream {
                ExecOutputStream::Stdout | ExecOutputStream::Pty => {
                    self.stdout.push(chunk.chunk.as_slice());
                }
                ExecOutputStream::Stderr => {
                    self.stderr.push(chunk.chunk.as_slice());
                }
            }
        }
        self.after_seq = response.next_seq.checked_sub(1).or(self.after_seq);
        self.exit_code = response.exit_code.or(self.exit_code);
    }

    fn response(
        &self,
        process_id: i32,
        truncation_policy: TruncationPolicy,
    ) -> UnifiedExecResponse {
        let stdout = output_stream(self.stdout.clone(), truncation_policy);
        let stderr = output_stream(self.stderr.clone(), truncation_policy);
        let aggregated_output = aggregate_streams(&stdout, &stderr, truncation_policy);
        UnifiedExecResponse {
            process_id,
            stdout,
            stderr,
            aggregated_output,
            exit_code: self.exit_code,
            timed_out: false,
        }
    }
}
