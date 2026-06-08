use http::HeaderMap;
use http::StatusCode;

const REQUEST_ID_HEADER: &str = "x-request-id";
const OAI_REQUEST_ID_HEADER: &str = "x-oai-request-id";
const CF_RAY_HEADER: &str = "cf-ray";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResponseDebugContext {
    pub request_id: Option<String>,
    pub cf_ray: Option<String>,
}

impl ResponseDebugContext {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let extract_header = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };

        Self {
            request_id: extract_header(REQUEST_ID_HEADER)
                .or_else(|| extract_header(OAI_REQUEST_ID_HEADER)),
            cf_ray: extract_header(CF_RAY_HEADER),
        }
    }
}

pub fn http_status_debug_message(status: StatusCode) -> String {
    format!("http {}", status.as_u16())
}

pub fn transport_debug_message(error: &(dyn std::error::Error + 'static)) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use pretty_assertions::assert_eq;

    #[test]
    fn extracts_provider_neutral_debug_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-oai-request-id", HeaderValue::from_static("req-1"));
        headers.insert("cf-ray", HeaderValue::from_static("ray-1"));

        assert_eq!(
            ResponseDebugContext::from_headers(&headers),
            ResponseDebugContext {
                request_id: Some("req-1".to_string()),
                cf_ray: Some("ray-1".to_string()),
            }
        );
    }

    #[test]
    fn status_message_omits_response_body() {
        assert_eq!(
            http_status_debug_message(StatusCode::UNAUTHORIZED),
            "http 401"
        );
    }
}
