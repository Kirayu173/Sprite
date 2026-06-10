use std::collections::HashMap;
use std::path::PathBuf;

use clap::Parser;
use exec::SpriteExecRequest;
use exec_server::ExecEnvPolicy;
use exec_server::ExecServerRuntimePaths;
use exec_server::TerminalSize;
use runtime_protocol::config_types::ShellEnvironmentPolicy;

#[derive(Debug, Parser)]
#[command(name = "sprite-exec")]
#[command(about = "Run a local command through Sprite's exec backend.")]
struct Cli {
    #[arg(long)]
    cwd: Option<PathBuf>,

    #[arg(long)]
    tty: bool,

    #[arg(long)]
    pipe_stdin: bool,

    #[arg(long)]
    arg0: Option<String>,

    #[arg(required = true, trailing_var_arg = true)]
    command: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("SPRITE_EXEC_LOG").unwrap_or_else(|_| "warn".to_string()))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let cwd = match cli.cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()?,
    };
    let runtime_paths = ExecServerRuntimePaths::new(std::env::current_exe()?, None)?;
    let output = exec::run_local_command(SpriteExecRequest {
        argv: cli.command,
        cwd,
        env: HashMap::new(),
        env_policy: Some(default_exec_env_policy()),
        tty: cli.tty,
        terminal_size: TerminalSize::default(),
        pipe_stdin: cli.pipe_stdin,
        arg0: cli.arg0,
        runtime_paths,
    })
    .await?;

    if !output.stdout.is_empty() {
        use std::io::Write as _;
        std::io::stdout().write_all(&output.stdout)?;
    }
    if !output.stderr.is_empty() {
        use std::io::Write as _;
        std::io::stderr().write_all(&output.stderr)?;
    }
    std::process::exit(output.exit_code);
}

fn default_exec_env_policy() -> ExecEnvPolicy {
    let policy = ShellEnvironmentPolicy::default();
    ExecEnvPolicy {
        inherit: policy.inherit,
        ignore_default_excludes: policy.ignore_default_excludes,
        exclude: policy
            .exclude
            .into_iter()
            .map(|pattern| pattern.to_string())
            .collect(),
        r#set: policy.r#set,
        include_only: policy
            .include_only
            .into_iter()
            .map(|pattern| pattern.to_string())
            .collect(),
    }
}
