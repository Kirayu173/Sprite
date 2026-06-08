use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProviderAuthError {
    pub reason: ProviderAuthErrorReason,
    pub message: String,
}

impl ProviderAuthError {
    pub fn new(reason: ProviderAuthErrorReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAuthErrorReason {
    Expired,
    Exhausted,
    Revoked,
    Other,
}
