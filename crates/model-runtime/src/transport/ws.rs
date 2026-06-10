use async_trait::async_trait;
use futures::SinkExt;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::HeaderMap;
use http::StatusCode;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;

use super::TransportError;

pub type WsMessageStream = BoxStream<'static, Result<WsMessage, TransportError>>;

#[derive(Debug, Clone)]
pub struct WsRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub connect_timeout: Option<Duration>,
}

#[derive(Debug)]
pub struct WsResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
}

pub struct WsConnectResponse {
    pub connection: Box<dyn WebsocketConnection>,
    pub response: WsResponse,
    pub messages: WsMessageStream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Close {
        code: Option<u16>,
        reason: Option<String>,
    },
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

#[async_trait]
pub trait WebsocketConnection: Send {
    async fn send_text(&mut self, text: String) -> Result<(), TransportError>;
}

#[async_trait]
pub trait WebsocketTransport: Send + Sync {
    async fn connect(&self, request: WsRequest) -> Result<WsConnectResponse, TransportError>;
}

#[derive(Clone, Debug, Default)]
pub struct ReqwestWebsocketTransport;

type RawWsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct TungsteniteConnection {
    sink: futures::stream::SplitSink<RawWsStream, Message>,
}

#[async_trait]
impl WebsocketConnection for TungsteniteConnection {
    async fn send_text(&mut self, text: String) -> Result<(), TransportError> {
        self.sink
            .send(Message::Text(text.into()))
            .await
            .map_err(map_ws_error)
    }
}

#[async_trait]
impl WebsocketTransport for ReqwestWebsocketTransport {
    async fn connect(&self, request: WsRequest) -> Result<WsConnectResponse, TransportError> {
        let mut ws_request = request
            .url
            .into_client_request()
            .map_err(|err| TransportError::Build(err.to_string()))?;
        ws_request.headers_mut().extend(request.headers);

        let connect = connect_async(ws_request);
        let (stream, response) = match request.connect_timeout {
            Some(connect_timeout) => timeout(connect_timeout, connect)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(map_ws_error)?,
            None => connect.await.map_err(map_ws_error)?,
        };

        let status = response.status();
        let headers = response.headers().clone();
        let (sink, stream) = stream.split();
        let messages =
            Box::pin(stream.map(|message| message.map(map_message).map_err(map_ws_error)));

        Ok(WsConnectResponse {
            connection: Box::new(TungsteniteConnection { sink }),
            response: WsResponse { status, headers },
            messages,
        })
    }
}

fn map_message(message: Message) -> WsMessage {
    match message {
        Message::Text(text) => WsMessage::Text(text.to_string()),
        Message::Binary(bytes) => WsMessage::Binary(bytes.to_vec()),
        Message::Close(frame) => WsMessage::Close {
            code: frame.as_ref().map(|frame| u16::from(frame.code)),
            reason: frame.map(|frame| frame.reason.to_string()),
        },
        Message::Ping(bytes) => WsMessage::Ping(bytes.to_vec()),
        Message::Pong(bytes) => WsMessage::Pong(bytes.to_vec()),
        Message::Frame(frame) => WsMessage::Binary(frame.payload().to_vec()),
    }
}

fn map_ws_error(error: tokio_tungstenite::tungstenite::Error) -> TransportError {
    match error {
        tokio_tungstenite::tungstenite::Error::Io(err) => TransportError::Network(err.to_string()),
        tokio_tungstenite::tungstenite::Error::Http(response) => TransportError::Http {
            status: response.status(),
            body: response
                .body()
                .as_ref()
                .map(|body| String::from_utf8_lossy(body).into_owned())
                .unwrap_or_default(),
        },
        other => TransportError::Network(other.to_string()),
    }
}
