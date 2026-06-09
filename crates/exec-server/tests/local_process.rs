use std::collections::HashMap;

use exec_server::Environment;
use exec_server::ExecOutputStream;
use exec_server::ExecParams;
use exec_server::ProcessId;
use exec_server::TerminalSize;
use exec_server::WriteStatus;

#[tokio::test]
async fn local_process_runs_command_and_captures_output() {
    let environment = Environment::default_for_tests();
    let process = environment
        .get_exec_backend()
        .start(ExecParams {
            process_id: ProcessId::from("local-echo"),
            argv: if cfg!(windows) {
                vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "echo sprite-local-ok".to_string(),
                ]
            } else {
                vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "echo sprite-local-ok".to_string(),
                ]
            },
            cwd: std::env::current_dir().expect("current dir"),
            env_policy: None,
            env: HashMap::new(),
            tty: false,
            terminal_size: TerminalSize::default(),
            pipe_stdin: false,
            arg0: None,
        })
        .await
        .expect("start process")
        .process;

    let mut after_seq = None;
    let mut stdout = Vec::new();
    loop {
        let response = process
            .read(after_seq, None, Some(1_000))
            .await
            .expect("read process");
        for chunk in response.chunks {
            if matches!(chunk.stream, ExecOutputStream::Stdout) {
                stdout.extend_from_slice(chunk.chunk.as_slice());
            }
            after_seq = Some(chunk.seq);
        }
        if response.closed {
            break;
        }
        after_seq = response.next_seq.checked_sub(1).or(after_seq);
    }

    assert!(String::from_utf8_lossy(&stdout).contains("sprite-local-ok"));
}

#[tokio::test]
async fn local_process_accepts_stdin_when_piped() {
    let environment = Environment::default_for_tests();
    let process = environment
        .get_exec_backend()
        .start(ExecParams {
            process_id: ProcessId::from("local-stdin"),
            argv: if cfg!(windows) {
                vec![
                    "powershell".to_string(),
                    "-NoProfile".to_string(),
                    "-Command".to_string(),
                    "$input | ForEach-Object { $_ }".to_string(),
                ]
            } else {
                vec!["cat".to_string()]
            },
            cwd: std::env::current_dir().expect("current dir"),
            env_policy: None,
            env: HashMap::new(),
            tty: false,
            terminal_size: TerminalSize::default(),
            pipe_stdin: true,
            arg0: None,
        })
        .await
        .expect("start process")
        .process;

    let write = process
        .write(b"sprite-stdin-ok\n".to_vec())
        .await
        .expect("write stdin");
    assert_eq!(write.status, WriteStatus::Accepted);

    process.terminate().await.expect("terminate process");
}
