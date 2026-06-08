/// Account state returned by a model provider before it is adapted to an app-facing wire type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAccount {
    /// Provider-level static key authentication, not tied to any first-party provider.
    ProviderKey,
    Authenticated {
        provider_name: String,
        account_id: Option<String>,
        email: Option<String>,
        label: Option<String>,
    },
    External {
        provider_name: String,
    },
}
