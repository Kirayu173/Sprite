use runtime_protocol::protocol::W3cTraceContext;
use tracing::Span;
use tracing::field;
use tracing::info_span;

pub fn app_server_request_span(
    method: &str,
    transport: &'static str,
    request_id: &impl std::fmt::Display,
    connection_id: &impl std::fmt::Display,
    parent_trace: Option<&W3cTraceContext>,
) -> Span {
    let span = info_span!(
        "app_server.request",
        otel.kind = "server",
        otel.name = method,
        rpc.system = "jsonrpc",
        rpc.method = method,
        rpc.transport = transport,
        rpc.request_id = %request_id,
        app_server.connection_id = %connection_id,
        app_server.api_version = "v2",
        app_server.client_name = field::Empty,
        app_server.client_version = field::Empty,
        turn.id = field::Empty,
    );

    attach_parent_context(&span, method, request_id, parent_trace);
    span
}

pub fn record_client_info(span: &Span, client_name: Option<&str>, client_version: Option<&str>) {
    if let Some(client_name) = client_name {
        span.record("app_server.client_name", client_name);
    }
    if let Some(client_version) = client_version {
        span.record("app_server.client_version", client_version);
    }
}

fn attach_parent_context(
    span: &Span,
    method: &str,
    request_id: &impl std::fmt::Display,
    parent_trace: Option<&W3cTraceContext>,
) {
    if let Some(trace) = parent_trace {
        if !crate::set_parent_from_w3c_trace_context(span, trace) {
            tracing::warn!(
                rpc_method = method,
                rpc_request_id = %request_id,
                "ignoring invalid inbound request trace carrier"
            );
        }
    } else if let Some(context) = crate::traceparent_context_from_env() {
        crate::set_parent_from_context(span, context);
    }
}
