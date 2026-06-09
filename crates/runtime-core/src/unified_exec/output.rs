use runtime_protocol::exec_output::StreamOutput;
use runtime_protocol::protocol::TruncationPolicy;

use crate::exec::HeadTailBuffer;

pub(super) fn output_stream(
    buffer: HeadTailBuffer,
    policy: TruncationPolicy,
) -> StreamOutput<String> {
    let (text, truncated_after_lines) = buffer.into_text();
    StreamOutput {
        text: utils_output_truncation::truncate_text(&text, policy),
        truncated_after_lines,
    }
}

pub(super) fn aggregate_streams(
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
