use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

use super::TransportError;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u64,
    pub base_delay: Duration,
    pub retry_429: bool,
    pub retry_5xx: bool,
    pub retry_transport: bool,
}

impl RetryPolicy {
    pub fn should_retry(&self, err: &TransportError, attempt: u64) -> bool {
        if attempt >= self.max_attempts {
            return false;
        }
        match err {
            TransportError::Http { status, .. } => {
                (self.retry_429 && status.as_u16() == 429)
                    || (self.retry_5xx && status.is_server_error())
            }
            TransportError::Timeout | TransportError::Network(_) => self.retry_transport,
            _ => false,
        }
    }
}

pub async fn run_with_retry<T, F, Fut>(policy: &RetryPolicy, mut op: F) -> Result<T, TransportError>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = Result<T, TransportError>>,
{
    for attempt in 0..=policy.max_attempts {
        match op(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) if policy.should_retry(&error, attempt) => {
                sleep(backoff(policy.base_delay, attempt + 1)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(TransportError::RetryLimit)
}

fn backoff(base: Duration, attempt: u64) -> Duration {
    let exp = 2u64.saturating_pow(attempt.saturating_sub(1) as u32);
    Duration::from_millis((base.as_millis() as u64).saturating_mul(exp.max(1)))
}
