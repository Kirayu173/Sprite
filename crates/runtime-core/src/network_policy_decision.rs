use execpolicy::{
    Decision as ExecPolicyDecision, NetworkRuleProtocol as ExecPolicyNetworkRuleProtocol,
};
use runtime_protocol::approvals::{
    NetworkApprovalContext, NetworkApprovalProtocol, NetworkPolicyAmendment,
    NetworkPolicyRuleAction,
};
use runtime_protocol::network_policy::{NetworkPolicyDecision, NetworkPolicyDecisionPayload};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedNetworkRequest {
    pub host: String,
    pub reason: String,
    pub decision: Option<NetworkPolicyDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPolicyNetworkRuleAmendment {
    pub protocol: ExecPolicyNetworkRuleProtocol,
    pub decision: ExecPolicyDecision,
    pub justification: String,
}

pub fn network_approval_context_from_payload(
    payload: &NetworkPolicyDecisionPayload,
) -> Option<NetworkApprovalContext> {
    if !payload.is_ask_from_decider() {
        return None;
    }
    let protocol = payload.protocol?;
    let host = payload.host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(NetworkApprovalContext {
        host: host.to_string(),
        protocol,
    })
}

pub fn denied_network_policy_message(blocked: &BlockedNetworkRequest) -> Option<String> {
    if blocked.decision != Some(NetworkPolicyDecision::Deny) {
        return None;
    }
    let host = blocked.host.trim();
    if host.is_empty() {
        return Some("Network access was blocked by policy.".to_string());
    }
    let detail = match blocked.reason.as_str() {
        "denied" => "domain is explicitly denied by policy and cannot be approved from this prompt",
        "not_allowed" => "domain is not on the allowlist for the current sandbox mode",
        "not_allowed_local" => "local/private network addresses are blocked by the sandbox policy",
        "method_not_allowed" => "request method is blocked by the current network mode",
        "proxy_disabled" => "network proxy is disabled",
        _ => "request is blocked by network policy",
    };

    Some(format!(
        "Network access to \"{host}\" was blocked: {detail}."
    ))
}

pub fn execpolicy_network_rule_amendment(
    amendment: &NetworkPolicyAmendment,
    network_approval_context: &NetworkApprovalContext,
    host: &str,
) -> ExecPolicyNetworkRuleAmendment {
    let protocol = match network_approval_context.protocol {
        NetworkApprovalProtocol::Http => ExecPolicyNetworkRuleProtocol::Http,
        NetworkApprovalProtocol::Https => ExecPolicyNetworkRuleProtocol::Https,
        NetworkApprovalProtocol::Socks5Tcp => ExecPolicyNetworkRuleProtocol::Socks5Tcp,
        NetworkApprovalProtocol::Socks5Udp => ExecPolicyNetworkRuleProtocol::Socks5Udp,
    };
    let (decision, action_verb) = match amendment.action {
        NetworkPolicyRuleAction::Allow => (ExecPolicyDecision::Allow, "Allow"),
        NetworkPolicyRuleAction::Deny => (ExecPolicyDecision::Forbidden, "Deny"),
    };
    let protocol_label = match network_approval_context.protocol {
        NetworkApprovalProtocol::Http => "http",
        NetworkApprovalProtocol::Https => "https_connect",
        NetworkApprovalProtocol::Socks5Tcp => "socks5_tcp",
        NetworkApprovalProtocol::Socks5Udp => "socks5_udp",
    };

    ExecPolicyNetworkRuleAmendment {
        protocol,
        decision,
        justification: format!("{action_verb} {protocol_label} access to {host}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_protocol::network_policy::NetworkDecisionSource;

    #[test]
    fn ask_payload_yields_network_context() {
        let payload = NetworkPolicyDecisionPayload {
            decision: NetworkPolicyDecision::Ask,
            source: NetworkDecisionSource::Decider,
            protocol: Some(NetworkApprovalProtocol::Https),
            host: Some("example.com".to_string()),
            reason: None,
            port: Some(443),
        };
        let context = network_approval_context_from_payload(&payload).unwrap();
        assert_eq!(context.host, "example.com");
    }

    #[test]
    fn denied_message_uses_reason_specific_detail() {
        let blocked = BlockedNetworkRequest {
            host: "example.com".to_string(),
            reason: "not_allowed".to_string(),
            decision: Some(NetworkPolicyDecision::Deny),
        };
        assert!(
            denied_network_policy_message(&blocked)
                .unwrap()
                .contains("allowlist")
        );
    }
}
