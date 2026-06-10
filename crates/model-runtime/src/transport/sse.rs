use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;

use futures::StreamExt;

use super::ByteStream;
use super::TransportError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

pub fn sse_event_stream(
    stream: ByteStream,
    idle_timeout: Duration,
) -> UnboundedReceiverStream<Result<SseEvent, TransportError>> {
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let mut stream = stream;
        let mut parser = SseParser::default();

        loop {
            match timeout(idle_timeout, stream.next()).await {
                Ok(Some(Ok(chunk))) => {
                    if let Err(error) = parser.push_chunk(&chunk, &tx) {
                        let _ = tx.send(Err(error));
                        return;
                    }
                }
                Ok(Some(Err(error))) => {
                    let _ = tx.send(Err(error));
                    return;
                }
                Ok(None) => {
                    if let Err(error) = parser.finish(&tx) {
                        let _ = tx.send(Err(error));
                    }
                    return;
                }
                Err(_) => {
                    let _ = tx.send(Err(TransportError::Timeout));
                    return;
                }
            }
        }
    });

    UnboundedReceiverStream::new(rx)
}

#[derive(Debug, Default)]
struct SseParser {
    pending_line: String,
    event_name: Option<String>,
    data_lines: Vec<String>,
}

impl SseParser {
    fn push_chunk(
        &mut self,
        chunk: &[u8],
        tx: &mpsc::UnboundedSender<Result<SseEvent, TransportError>>,
    ) -> Result<(), TransportError> {
        let text = String::from_utf8_lossy(chunk);
        self.pending_line.push_str(&text);

        while let Some(pos) = self.pending_line.find('\n') {
            let mut line = self.pending_line[..pos].to_string();
            self.pending_line.drain(..=pos);
            if line.ends_with('\r') {
                line.pop();
            }

            if line.is_empty() {
                self.flush_event(tx)?;
                continue;
            }

            if let Some(rest) = line.strip_prefix("event:") {
                self.event_name = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                self.data_lines.push(rest.trim_start().to_string());
            }
        }

        Ok(())
    }

    fn finish(
        &mut self,
        tx: &mpsc::UnboundedSender<Result<SseEvent, TransportError>>,
    ) -> Result<(), TransportError> {
        if !self.pending_line.trim().is_empty() {
            if let Some(rest) = self.pending_line.strip_prefix("data:") {
                self.data_lines.push(rest.trim_start().to_string());
            }
            self.pending_line.clear();
        }
        self.flush_event(tx)
    }

    fn flush_event(
        &mut self,
        tx: &mpsc::UnboundedSender<Result<SseEvent, TransportError>>,
    ) -> Result<(), TransportError> {
        if self.event_name.is_none() && self.data_lines.is_empty() {
            return Ok(());
        }
        tx.send(Ok(SseEvent {
            event: self.event_name.take(),
            data: self.data_lines.join("\n"),
        }))
        .map_err(|_| TransportError::Network("sse consumer dropped".to_string()))?;
        self.data_lines.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::stream;
    use tokio_stream::StreamExt;

    use super::*;

    #[tokio::test]
    async fn sse_parser_joins_multiline_data() {
        let stream = stream::iter(vec![Ok(Bytes::from(
            "event: test\ndata: hello\ndata: world\n\n",
        ))]);
        let mut events = sse_event_stream(Box::pin(stream), Duration::from_millis(50));

        let first = events.next().await.expect("event").expect("parsed event");

        assert_eq!(
            first,
            SseEvent {
                event: Some("test".to_string()),
                data: "hello\nworld".to_string(),
            }
        );
    }
}
