use std::collections::HashSet;

use runtime_protocol::approvals::{
    AutoReviewAssessmentAction, AutoReviewAssessmentDecisionSource, AutoReviewAssessmentEvent,
    AutoReviewAssessmentOutcome, AutoReviewAssessmentStatus, AutoReviewRiskLevel,
    AutoReviewUserAuthorization, NetworkApprovalContext, NetworkApprovalProtocol,
};
use runtime_protocol::config_types::ApprovalsReviewer;
use runtime_protocol::protocol::ReviewDecision;
use runtime_protocol::request_permissions::{
    PermissionGrantScope, RequestPermissionProfile, RequestPermissionsResponse,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct HostApprovalKey {
    host: String,
    protocol: &'static str,
    port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkApprovalRequest {
    pub target: String,
    pub host: String,
    pub protocol: NetworkApprovalProtocol,
    pub port: u16,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkApprovalOutcome {
    AllowOnce,
    AllowForSession,
    Deny,
    DenyByPolicy(String),
}

#[derive(Default)]
pub struct SessionNetworkApprovalCache {
    approved: HashSet<HostApprovalKey>,
    denied: HashSet<HostApprovalKey>,
}

impl SessionNetworkApprovalCache {
    pub fn is_approved(&self, request: &NetworkApprovalRequest) -> bool {
        self.approved.contains(&HostApprovalKey::from(request))
    }

    pub fn is_denied(&self, request: &NetworkApprovalRequest) -> bool {
        self.denied.contains(&HostApprovalKey::from(request))
    }

    pub fn allow_for_session(&mut self, request: &NetworkApprovalRequest) {
        let key = HostApprovalKey::from(request);
        self.denied.remove(&key);
        self.approved.insert(key);
    }

    pub fn deny_for_session(&mut self, request: &NetworkApprovalRequest) {
        let key = HostApprovalKey::from(request);
        self.approved.remove(&key);
        self.denied.insert(key);
    }
}

impl From<&NetworkApprovalRequest> for HostApprovalKey {
    fn from(value: &NetworkApprovalRequest) -> Self {
        Self {
            host: value.host.to_ascii_lowercase(),
            protocol: protocol_label(value.protocol),
            port: value.port,
        }
    }
}

fn protocol_label(protocol: NetworkApprovalProtocol) -> &'static str {
    match protocol {
        NetworkApprovalProtocol::Http => "http",
        NetworkApprovalProtocol::Https => "https",
        NetworkApprovalProtocol::Socks5Tcp => "socks5_tcp",
        NetworkApprovalProtocol::Socks5Udp => "socks5_udp",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewerRequest {
    Command {
        command: Vec<String>,
        cwd: String,
    },
    Network(NetworkApprovalRequest),
    ApplyPatch {
        cwd: String,
        files: Vec<String>,
    },
    Permissions {
        reason: Option<String>,
        permissions: RequestPermissionProfile,
    },
}

pub trait AutomatedReviewer {
    fn review(&self, request: &ReviewerRequest) -> ReviewDecision;
}

#[derive(Default)]
pub struct SpriteAutomatedReviewer;

impl AutomatedReviewer for SpriteAutomatedReviewer {
    fn review(&self, request: &ReviewerRequest) -> ReviewDecision {
        match request {
            ReviewerRequest::Command { command, .. } => {
                let dangerous =
                    shell_command::is_dangerous_command::command_might_be_dangerous(command);
                if dangerous {
                    ReviewDecision::Denied
                } else {
                    ReviewDecision::Approved
                }
            }
            ReviewerRequest::Network(request) => {
                if is_private_host(&request.host) {
                    ReviewDecision::Denied
                } else {
                    ReviewDecision::Approved
                }
            }
            ReviewerRequest::ApplyPatch { files, .. } => {
                if files.is_empty() {
                    ReviewDecision::Denied
                } else {
                    ReviewDecision::Approved
                }
            }
            ReviewerRequest::Permissions { permissions, .. } => {
                if permissions.is_empty() {
                    ReviewDecision::Denied
                } else {
                    ReviewDecision::ApprovedForSession
                }
            }
        }
    }
}

pub struct NetworkApprovalService<R: AutomatedReviewer> {
    reviewer: R,
    cache: SessionNetworkApprovalCache,
}

impl<R: AutomatedReviewer> NetworkApprovalService<R> {
    pub fn new(reviewer: R) -> Self {
        Self {
            reviewer,
            cache: SessionNetworkApprovalCache::default(),
        }
    }

    pub fn cache(&self) -> &SessionNetworkApprovalCache {
        &self.cache
    }

    pub fn cache_mut(&mut self) -> &mut SessionNetworkApprovalCache {
        &mut self.cache
    }

    pub fn evaluate(
        &mut self,
        request: &NetworkApprovalRequest,
        reviewer: ApprovalsReviewer,
    ) -> NetworkApprovalOutcome {
        if self.cache.is_denied(request) {
            return NetworkApprovalOutcome::DenyByPolicy(
                "network access was previously denied for this session".to_string(),
            );
        }
        if self.cache.is_approved(request) {
            return NetworkApprovalOutcome::AllowForSession;
        }
        if matches!(reviewer, ApprovalsReviewer::User) {
            return NetworkApprovalOutcome::AllowOnce;
        }

        match self
            .reviewer
            .review(&ReviewerRequest::Network(request.clone()))
        {
            ReviewDecision::Approved => NetworkApprovalOutcome::AllowOnce,
            ReviewDecision::ApprovedForSession => {
                self.cache.allow_for_session(request);
                NetworkApprovalOutcome::AllowForSession
            }
            ReviewDecision::Denied | ReviewDecision::Abort | ReviewDecision::TimedOut => {
                self.cache.deny_for_session(request);
                NetworkApprovalOutcome::Deny
            }
            ReviewDecision::ApprovedExecpolicyAmendment { .. }
            | ReviewDecision::NetworkPolicyAmendment { .. } => NetworkApprovalOutcome::AllowOnce,
        }
    }
}

pub fn review_permissions_request<R: AutomatedReviewer>(
    reviewer: &R,
    reason: Option<String>,
    permissions: RequestPermissionProfile,
) -> RequestPermissionsResponse {
    let decision = reviewer.review(&ReviewerRequest::Permissions {
        reason,
        permissions: permissions.clone(),
    });
    let scope = if matches!(decision, ReviewDecision::ApprovedForSession) {
        PermissionGrantScope::Session
    } else {
        PermissionGrantScope::Turn
    };
    RequestPermissionsResponse {
        permissions,
        scope,
        strict_auto_review: false,
    }
}

pub fn build_auto_review_started_event(
    id: String,
    turn_id: String,
    target_item_id: Option<String>,
    action: AutoReviewAssessmentAction,
) -> AutoReviewAssessmentEvent {
    AutoReviewAssessmentEvent {
        id,
        target_item_id,
        turn_id,
        started_at_ms: 0,
        completed_at_ms: None,
        status: AutoReviewAssessmentStatus::InProgress,
        risk_level: None,
        user_authorization: None,
        rationale: None,
        decision_source: None,
        action,
    }
}

pub fn build_auto_review_completed_event(
    mut started: AutoReviewAssessmentEvent,
    outcome: AutoReviewAssessmentOutcome,
    rationale: String,
) -> AutoReviewAssessmentEvent {
    started.completed_at_ms = Some(0);
    started.status = match outcome {
        AutoReviewAssessmentOutcome::Allow => AutoReviewAssessmentStatus::Approved,
        AutoReviewAssessmentOutcome::Deny => AutoReviewAssessmentStatus::Denied,
    };
    started.risk_level = Some(match outcome {
        AutoReviewAssessmentOutcome::Allow => AutoReviewRiskLevel::Low,
        AutoReviewAssessmentOutcome::Deny => AutoReviewRiskLevel::High,
    });
    started.user_authorization = Some(match outcome {
        AutoReviewAssessmentOutcome::Allow => AutoReviewUserAuthorization::High,
        AutoReviewAssessmentOutcome::Deny => AutoReviewUserAuthorization::Unknown,
    });
    started.rationale = Some(rationale);
    started.decision_source = Some(AutoReviewAssessmentDecisionSource::Agent);
    started
}

pub fn network_approval_context(request: &NetworkApprovalRequest) -> NetworkApprovalContext {
    NetworkApprovalContext {
        host: request.host.clone(),
        protocol: request.protocol,
    }
}

fn is_private_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    ) || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_reviewer_denies_private_network_access() {
        let backend = SpriteAutomatedReviewer;
        let mut service = NetworkApprovalService::new(backend);
        let request = NetworkApprovalRequest {
            target: "http://127.0.0.1:8080".to_string(),
            host: "127.0.0.1".to_string(),
            protocol: NetworkApprovalProtocol::Http,
            port: 8080,
            reason: None,
        };
        assert_eq!(
            service.evaluate(&request, ApprovalsReviewer::AutoReview),
            NetworkApprovalOutcome::Deny
        );
    }

    #[test]
    fn permissions_review_preserves_requested_profile() {
        let reviewer = SpriteAutomatedReviewer;
        let permissions = RequestPermissionProfile {
            network: Some(runtime_protocol::models::NetworkPermissions {
                enabled: Some(true),
            }),
            file_system: None,
        };
        let response = review_permissions_request(&reviewer, None, permissions.clone());
        assert_eq!(response.permissions, permissions);
    }
}
