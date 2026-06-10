use http::HeaderMap;
use http::HeaderValue;
use http::header::ACCEPT;
use http::header::AUTHORIZATION;
use http::header::CONTENT_TYPE;
use model_runtime::ModelRuntimeError;
use model_runtime::ModelTurnRequest;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRequestMetadata {
    pub headers: HeaderMap,
    pub client_metadata: Option<HashMap<String, String>>,
    pub previous_response_id: Option<String>,
}

pub fn build_sse_headers(
    base_headers: HeaderMap,
    auth_header: Option<HeaderValue>,
    request: &ModelTurnRequest,
) -> Result<ResolvedRequestMetadata, ModelRuntimeError> {
    build_headers(
        base_headers,
        auth_header,
        request,
        HeaderValue::from_static("text/event-stream"),
    )
}

pub fn build_ws_headers(
    base_headers: HeaderMap,
    auth_header: Option<HeaderValue>,
    request: &ModelTurnRequest,
) -> Result<ResolvedRequestMetadata, ModelRuntimeError> {
    build_headers(
        base_headers,
        auth_header,
        request,
        HeaderValue::from_static("application/json"),
    )
}

fn build_headers(
    mut headers: HeaderMap,
    auth_header: Option<HeaderValue>,
    request: &ModelTurnRequest,
    accept: HeaderValue,
) -> Result<ResolvedRequestMetadata, ModelRuntimeError> {
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, accept);
    if let Some(auth_header) = auth_header {
        headers.insert(AUTHORIZATION, auth_header);
    }

    let mut client_metadata = request.metadata.clone();
    move_metadata_header(
        &mut headers,
        &mut client_metadata,
        "session_id",
        "session-id",
    )?;
    move_metadata_header(
        &mut headers,
        &mut client_metadata,
        "conversation_id",
        "conversation-id",
    )?;
    move_metadata_header(&mut headers, &mut client_metadata, "thread_id", "thread-id")?;
    move_metadata_header(
        &mut headers,
        &mut client_metadata,
        "turn_state",
        "x-codex-turn-state",
    )?;
    move_metadata_header(
        &mut headers,
        &mut client_metadata,
        "traceparent",
        "traceparent",
    )?;
    move_metadata_header(
        &mut headers,
        &mut client_metadata,
        "tracestate",
        "tracestate",
    )?;

    let previous_response_id = request
        .previous_response_id
        .clone()
        .or_else(|| client_metadata.remove("previous_response_id"));

    if let Some(personality) = request.personality {
        client_metadata
            .entry("personality".to_string())
            .or_insert_with(|| personality.to_string());
    }

    Ok(ResolvedRequestMetadata {
        headers,
        client_metadata: (!client_metadata.is_empty()).then_some(client_metadata),
        previous_response_id,
    })
}

fn move_metadata_header(
    headers: &mut HeaderMap,
    metadata: &mut HashMap<String, String>,
    key: &str,
    header_name: &str,
) -> Result<(), ModelRuntimeError> {
    let Some(value) = metadata.remove(key) else {
        return Ok(());
    };
    let header_value = HeaderValue::from_str(&value)
        .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
    let name = header_name
        .parse::<http::HeaderName>()
        .map_err(|err| ModelRuntimeError::InvalidRequest(err.to_string()))?;
    headers.insert(name, header_value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_sse_headers_promotes_reserved_metadata_to_headers() {
        let mut request = ModelTurnRequest::new(
            model_provider_info::ModelProviderInfo::create_openai_compatible_provider(Some(
                "https://example.test/v1".to_string(),
            )),
            "demo",
        );
        request
            .metadata
            .insert("session_id".to_string(), "session-1".to_string());
        request
            .metadata
            .insert("conversation_id".to_string(), "thread-9".to_string());
        request
            .metadata
            .insert("traceparent".to_string(), "00-abc-def-01".to_string());
        request.previous_response_id = Some("resp-1".to_string());

        let resolved = build_sse_headers(HeaderMap::new(), None, &request).expect("headers");

        assert_eq!(
            resolved
                .headers
                .get("session-id")
                .and_then(|value| value.to_str().ok()),
            Some("session-1")
        );
        assert_eq!(
            resolved
                .headers
                .get("conversation-id")
                .and_then(|value| value.to_str().ok()),
            Some("thread-9")
        );
        assert_eq!(
            resolved
                .headers
                .get("traceparent")
                .and_then(|value| value.to_str().ok()),
            Some("00-abc-def-01")
        );
        assert_eq!(resolved.previous_response_id.as_deref(), Some("resp-1"));
        assert_eq!(resolved.client_metadata, None);
    }
}
