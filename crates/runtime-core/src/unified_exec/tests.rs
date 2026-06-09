use std::collections::HashMap;

use exec_server::ExecServerRuntimePaths;
use exec_server::TerminalSize;
use runtime_protocol::protocol::TruncationPolicy;
use utils_absolute_path::AbsolutePathBuf;

use super::*;

fn runtime_paths() -> ExecServerRuntimePaths {
    ExecServerRuntimePaths::new(std::env::current_exe().unwrap(), None).unwrap()
}

fn current_dir() -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(std::env::current_dir().unwrap()).unwrap()
}

fn shell_command() -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".to_string(), "/q".to_string()]
    } else {
        vec!["sh".to_string()]
    }
}

fn exec_request(process_id: i32, command: Vec<String>) -> ExecCommandRequest {
    ExecCommandRequest {
        command,
        process_id,
        yield_time_ms: 250,
        max_output_tokens: Some(1024),
        cwd: current_dir(),
        env: HashMap::new(),
        env_policy: None,
        tty: false,
        terminal_size: TerminalSize::default(),
        arg0: None,
        runtime_paths: runtime_paths(),
    }
}

#[tokio::test]
async fn starts_process_and_writes_stdin() {
    let manager = UnifiedExecProcessManager::default();

    let started = manager
        .exec_command(exec_request(42, shell_command()))
        .await
        .expect("start process");
    assert_eq!(started.process_id, 42);
    assert_eq!(started.exit_code, None);

    let input = if cfg!(windows) {
        "echo sprite-stdin-ok\r\nexit /b 0\r\n"
    } else {
        "echo sprite-stdin-ok\nexit 0\n"
    };
    let output = manager
        .write_stdin(WriteStdinRequest {
            process_id: 42,
            input: input.to_string(),
            yield_time_ms: 1_000,
            max_output_tokens: Some(1024),
            truncation_policy: TruncationPolicy::Tokens(1024),
        })
        .await
        .expect("write stdin");

    assert!(
        output.aggregated_output.text.contains("sprite-stdin-ok"),
        "{:?}",
        output.aggregated_output.text
    );
    assert_eq!(output.exit_code, Some(0));
}

#[tokio::test]
async fn terminates_stored_process() {
    let manager = UnifiedExecProcessManager::default();

    manager
        .exec_command(exec_request(43, shell_command()))
        .await
        .expect("start process");

    manager
        .terminate_process(43)
        .await
        .expect("terminate process");
    assert!(matches!(
        manager.terminate_process(43).await,
        Err(UnifiedExecError::UnknownProcess(43))
    ));
}

#[tokio::test]
async fn rejects_duplicate_process_id_without_losing_original() {
    let manager = UnifiedExecProcessManager::default();
    let command = shell_command();

    manager
        .exec_command(exec_request(44, command.clone()))
        .await
        .expect("start process");

    assert!(matches!(
        manager.exec_command(exec_request(44, command)).await,
        Err(UnifiedExecError::DuplicateProcess(44))
    ));

    manager
        .terminate_process(44)
        .await
        .expect("original process remains tracked");
}

#[tokio::test]
async fn yield_polls_accumulate_output_until_exit() {
    let manager = UnifiedExecProcessManager::default();

    manager
        .exec_command(exec_request(45, shell_command()))
        .await
        .expect("start process");

    let input = if cfg!(windows) {
        "echo first\r\necho second\r\nexit /b 0\r\n"
    } else {
        "echo first\necho second\nexit 0\n"
    };

    let output = manager
        .write_stdin(WriteStdinRequest {
            process_id: 45,
            input: input.to_string(),
            yield_time_ms: 1_000,
            max_output_tokens: Some(1024),
            truncation_policy: TruncationPolicy::Tokens(1024),
        })
        .await
        .expect("write stdin");

    assert!(output.aggregated_output.text.contains("first"));
    assert!(output.aggregated_output.text.contains("second"));
    assert_eq!(output.exit_code, Some(0));
}
