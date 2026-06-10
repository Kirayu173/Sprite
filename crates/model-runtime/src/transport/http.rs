use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::HeaderMap;
use http::Method;
use http::StatusCode;
use serde_json::Value;
use std::time::Duration;

use super::TransportError;

pub type ByteStream = BoxStream<'static, Result<Bytes, TransportError>>;

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Option<Value>,
    pub timeout: Option<Duration>,
}

impl HttpRequest {
    pub fn post_json(url: impl Into<String>, headers: HeaderMap, body: Value) -> Self {
        Self {
            method: Method::POST,
            url: url.into(),
            headers,
            body: Some(body),
            timeout: None,
        }
    }

    pub fn get(url: impl Into<String>, headers: HeaderMap) -> Self {
        Self {
            method: Method::GET,
            url: url.into(),
            headers,
            body: None,
            timeout: None,
        }
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

pub struct StreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub bytes: ByteStream,
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
    async fn stream(&self, request: HttpRequest) -> Result<StreamResponse, TransportError>;
}

#[derive(Clone, Debug, Default)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    fn build(&self, request: HttpRequest) -> reqwest::RequestBuilder {
        let mut builder = self.client.request(request.method, request.url);
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        builder = builder.headers(request.headers);
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }
        builder
    }

    pub(crate) fn map_error(error: reqwest::Error) -> TransportError {
        if error.is_timeout() {
            TransportError::Timeout
        } else {
            TransportError::Network(error.to_string())
        }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let response = self.build(request).send().await.map_err(Self::map_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.bytes().await.map_err(Self::map_error)?;
        if !status.is_success() {
            return Err(TransportError::Http {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    async fn stream(&self, request: HttpRequest) -> Result<StreamResponse, TransportError> {
        let response = self.build(request).send().await.map_err(Self::map_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read error body>".to_string());
            return Err(TransportError::Http { status, body });
        }
        Ok(StreamResponse {
            status,
            headers,
            bytes: Box::pin(
                response
                    .bytes_stream()
                    .map(|item| item.map_err(Self::map_error)),
            ),
        })
    }
}
