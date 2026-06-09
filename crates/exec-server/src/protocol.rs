use std::collections::HashMap;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use runtime_protocol::config_types::ShellEnvironmentPolicyInherit;
use serde::Deserialize;
use serde::Serialize;

use crate::ProcessId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ByteChunk(#[serde(with = "base64_bytes")] pub Vec<u8>);

impl ByteChunk {
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for ByteChunk {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfo {
    pub shell: ShellInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

impl From<TerminalSize> for utils_pty::TerminalSize {
    fn from(value: TerminalSize) -> Self {
        Self {
            rows: value.rows,
            cols: value.cols,
        }
    }
}

impl From<utils_pty::TerminalSize> for TerminalSize {
    fn from(value: utils_pty::TerminalSize) -> Self {
        Self {
            rows: value.rows,
            cols: value.cols,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecParams {
    pub process_id: ProcessId,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub env_policy: Option<ExecEnvPolicy>,
    pub env: HashMap<String, String>,
    pub tty: bool,
    #[serde(default)]
    pub terminal_size: TerminalSize,
    #[serde(default)]
    pub pipe_stdin: bool,
    pub arg0: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecEnvPolicy {
    pub inherit: ShellEnvironmentPolicyInherit,
    pub ignore_default_excludes: bool,
    pub exclude: Vec<String>,
    pub r#set: HashMap<String, String>,
    pub include_only: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResponse {
    pub process_id: ProcessId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadParams {
    pub process_id: ProcessId,
    pub after_seq: Option<u64>,
    pub max_bytes: Option<usize>,
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputChunk {
    pub seq: u64,
    pub stream: ExecOutputStream,
    pub chunk: ByteChunk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResponse {
    pub chunks: Vec<ProcessOutputChunk>,
    pub next_seq: u64,
    pub exited: bool,
    pub exit_code: Option<i32>,
    pub closed: bool,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteParams {
    pub process_id: ProcessId,
    pub chunk: ByteChunk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteStatus {
    Accepted,
    UnknownProcess,
    StdinClosed,
    Starting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResponse {
    pub status: WriteStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminateParams {
    pub process_id: ProcessId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminateResponse {
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeParams {
    pub process_id: ProcessId,
    pub size: TerminalSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeResponse {
    pub resized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecOutputStream {
    Stdout,
    Stderr,
    Pty,
}

mod base64_bytes {
    use super::BASE64_STANDARD;
    use base64::Engine as _;
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serializer;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&BASE64_STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        BASE64_STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}
