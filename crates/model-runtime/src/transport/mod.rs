mod http;
mod retry;
mod sse;
mod ws;

pub use http::ByteStream;
pub use http::HttpRequest;
pub use http::HttpResponse;
pub use http::HttpTransport;
pub use http::ReqwestTransport;
pub use http::StreamResponse;
pub use retry::RetryPolicy;
pub use retry::run_with_retry;
pub use sse::SseEvent;
pub use sse::sse_event_stream;
pub use ws::ReqwestWebsocketTransport;
pub use ws::WebsocketConnection;
pub use ws::WebsocketTransport;
pub use ws::WsConnectResponse;
pub use ws::WsMessage;
pub use ws::WsMessageStream;
pub use ws::WsRequest;
pub use ws::WsResponse;

use ::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("request could not be built: {0}")]
    Build(String),
    #[error("request timed out")]
    Timeout,
    #[error("network error: {0}")]
    Network(String),
    #[error("http error {status}: {body}")]
    Http { status: StatusCode, body: String },
    #[error("retry limit reached")]
    RetryLimit,
}
