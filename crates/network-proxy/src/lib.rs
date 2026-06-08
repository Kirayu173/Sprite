use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use globset::GlobBuilder;
use globset::GlobSet;
use globset::GlobSetBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::path::Path;
use url::Host as UrlHost;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Full,
    Limited,
}

impl NetworkMode {
    pub fn allows_method(self, method: &str) -> bool {
        match self {
            Self::Full => true,
            Self::Limited => matches!(
                method.to_ascii_uppercase().as_str(),
                "GET" | "HEAD" | "OPTIONS"
            ),
        }
    }
}

/// Variant order encodes effective precedence for duplicate patterns:
/// `None < Allow < Deny`, so deny wins over allow when entries conflict.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum NetworkDomainPermission {
    None,
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NetworkUnixSocketPermission {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkDomainPermissions {
    pub entries: Vec<NetworkDomainPermissionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkDomainPermissionEntry {
    pub pattern: String,
    pub permission: NetworkDomainPermission,
}

impl NetworkDomainPermissions {
    pub fn effective_entries(&self) -> Vec<NetworkDomainPermissionEntry> {
        let mut order = Vec::new();
        let mut effective = BTreeMap::new();
        for entry in &self.entries {
            if !effective.contains_key(&entry.pattern) {
                order.push(entry.pattern.clone());
            }
            let permission = effective
                .entry(entry.pattern.clone())
                .or_insert(entry.permission);
            if entry.permission > *permission {
                *permission = entry.permission;
            }
        }
        order
            .into_iter()
            .filter_map(|pattern| {
                effective
                    .remove(&pattern)
                    .map(|permission| NetworkDomainPermissionEntry {
                        pattern,
                        permission,
                    })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkUnixSocketPermissions {
    pub entries: BTreeMap<String, NetworkUnixSocketPermission>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InjectedHeaderConfig {
    pub name: String,
    pub secret_env_var: Option<String>,
    pub secret_file: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MitmHookBodyConfig(pub serde_json::Value);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MitmHookMatchConfig {
    pub methods: Vec<String>,
    pub path_prefixes: Vec<String>,
    pub query: BTreeMap<String, Vec<String>>,
    pub headers: BTreeMap<String, Vec<String>>,
    pub body: Option<MitmHookBodyConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MitmHookActionsConfig {
    pub strip_request_headers: Vec<String>,
    pub inject_request_headers: Vec<InjectedHeaderConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MitmHookConfig {
    pub host: String,
    pub matcher: MitmHookMatchConfig,
    pub actions: MitmHookActionsConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkProxyConfig {
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkConfig {
    pub enabled: bool,
    pub proxy_url: String,
    pub enable_socks5: bool,
    pub socks_url: String,
    pub enable_socks5_udp: bool,
    pub allow_upstream_proxy: bool,
    pub dangerously_allow_non_loopback_proxy: bool,
    pub dangerously_allow_all_unix_sockets: bool,
    pub mode: NetworkMode,
    pub domains: Option<NetworkDomainPermissions>,
    pub unix_sockets: Option<NetworkUnixSocketPermissions>,
    pub allow_local_binding: bool,
    pub mitm_hooks: Vec<MitmHookConfig>,
    pub mitm: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_url: String::new(),
            enable_socks5: false,
            socks_url: String::new(),
            enable_socks5_udp: false,
            allow_upstream_proxy: false,
            dangerously_allow_non_loopback_proxy: false,
            dangerously_allow_all_unix_sockets: false,
            mode: NetworkMode::Full,
            domains: None,
            unix_sockets: None,
            allow_local_binding: false,
            mitm_hooks: Vec::new(),
            mitm: false,
        }
    }
}

impl NetworkConfig {
    pub fn allowed_domains(&self) -> Option<Vec<String>> {
        self.domain_entries(NetworkDomainPermission::Allow)
    }

    pub fn denied_domains(&self) -> Option<Vec<String>> {
        self.domain_entries(NetworkDomainPermission::Deny)
    }

    fn domain_entries(&self, permission: NetworkDomainPermission) -> Option<Vec<String>> {
        self.domains
            .as_ref()
            .map(|domains| {
                domains
                    .effective_entries()
                    .into_iter()
                    .filter(|entry| entry.permission == permission)
                    .map(|entry| entry.pattern)
                    .collect()
            })
            .filter(|entries: &Vec<String>| !entries.is_empty())
    }

    pub fn allow_unix_sockets(&self) -> Vec<String> {
        self.unix_sockets
            .as_ref()
            .map(|unix_sockets| {
                unix_sockets
                    .entries
                    .iter()
                    .filter(|(_, permission)| {
                        matches!(permission, NetworkUnixSocketPermission::Allow)
                    })
                    .map(|(path, _)| path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn upsert_domain_permission(
        &mut self,
        pattern: String,
        permission: NetworkDomainPermission,
        normalize: impl Fn(&str) -> String,
    ) {
        let domains = self
            .domains
            .get_or_insert_with(NetworkDomainPermissions::default);
        let normalized = normalize(&pattern);
        domains
            .entries
            .retain(|entry| normalize(&entry.pattern) != normalized);
        domains.entries.push(NetworkDomainPermissionEntry {
            pattern,
            permission,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Host(String);

impl Host {
    pub fn parse(input: &str) -> Result<Self> {
        let normalized = normalize_host(input);
        ensure!(!normalized.is_empty(), "host is empty");
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn normalize_host(host: &str) -> String {
    let host = host.trim();
    if host.starts_with('[')
        && let Some(end) = host.find(']')
    {
        return normalize_dns_host_or_ip_literal(&host[1..end]);
    }

    if host.bytes().filter(|byte| *byte == b':').count() == 1 {
        let host = host.split(':').next().unwrap_or_default();
        return normalize_dns_host_or_ip_literal(host);
    }

    normalize_dns_host_or_ip_literal(host)
}

fn normalize_dns_host_or_ip_literal(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    if let Some(ip) = normalize_ip_literal(host) {
        return ip;
    }
    host.to_string()
}

fn normalize_ip_literal(host: &str) -> Option<String> {
    if host.parse::<IpAddr>().is_ok() {
        return Some(host.to_string());
    }
    for delimiter in ["%25", "%"] {
        if let Some((ip, scope)) = host.split_once(delimiter)
            && ip.parse::<IpAddr>().is_ok()
        {
            return Some(format!("{ip}%{scope}"));
        }
    }
    None
}

fn unscoped_ip_literal(host: &str) -> Option<&str> {
    let (ip, _) = host.split_once('%')?;
    ip.parse::<IpAddr>().ok()?;
    Some(ip)
}

pub fn is_loopback_host(host: &Host) -> bool {
    let host = unscoped_ip_literal(host.as_str()).unwrap_or(host.as_str());
    if host == "localhost" {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_non_public_ipv4(ip),
        IpAddr::V6(ip) => is_non_public_ipv6(ip),
    }
}

fn is_non_public_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ipv4_in_cidr(ip, [0, 0, 0, 0], 8)
        || ipv4_in_cidr(ip, [100, 64, 0, 0], 10)
        || ipv4_in_cidr(ip, [192, 0, 0, 0], 24)
        || ipv4_in_cidr(ip, [192, 0, 2, 0], 24)
        || ipv4_in_cidr(ip, [198, 18, 0, 0], 15)
        || ipv4_in_cidr(ip, [198, 51, 100, 0], 24)
        || ipv4_in_cidr(ip, [203, 0, 113, 0], 24)
        || ipv4_in_cidr(ip, [240, 0, 0, 0], 4)
}

fn ipv4_in_cidr(ip: Ipv4Addr, base: [u8; 4], prefix: u8) -> bool {
    let ip = u32::from(ip);
    let base = u32::from(Ipv4Addr::from(base));
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    (ip & mask) == (base & mask)
}

fn is_non_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4() {
        return is_non_public_ipv4(v4) || ip.is_loopback();
    }
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
}

fn normalize_pattern(pattern: &str) -> String {
    let pattern = pattern.trim();
    if pattern == "*" {
        return "*".to_string();
    }

    let (prefix, remainder) = if let Some(domain) = pattern.strip_prefix("**.") {
        ("**.", domain)
    } else if let Some(domain) = pattern.strip_prefix("*.") {
        ("*.", domain)
    } else {
        ("", pattern)
    };
    let remainder = normalize_host(remainder);
    if prefix.is_empty() {
        remainder
    } else {
        format!("{prefix}{remainder}")
    }
}

pub fn is_global_wildcard_domain_pattern(pattern: &str) -> bool {
    let normalized = normalize_pattern(pattern);
    expand_domain_pattern(&normalized)
        .iter()
        .any(|candidate| candidate == "*")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobalWildcard {
    Allow,
    Reject,
}

pub fn compile_allowlist_globset(patterns: &[String]) -> Result<GlobSet> {
    compile_globset_with_policy(patterns, GlobalWildcard::Allow)
}

pub fn compile_denylist_globset(patterns: &[String]) -> Result<GlobSet> {
    compile_globset_with_policy(patterns, GlobalWildcard::Reject)
}

fn compile_globset_with_policy(patterns: &[String], wildcard: GlobalWildcard) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut seen = HashSet::new();
    for pattern in patterns {
        if wildcard == GlobalWildcard::Reject && is_global_wildcard_domain_pattern(pattern) {
            bail!(
                "unsupported global wildcard domain pattern \"*\"; use exact hosts or scoped wildcards like *.example.com or **.example.com"
            );
        }
        for candidate in expand_domain_pattern(&normalize_pattern(pattern)) {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            let glob = GlobBuilder::new(&candidate)
                .case_insensitive(true)
                .build()?;
            builder.add(glob);
        }
    }
    Ok(builder.build()?)
}

#[derive(Debug, Clone)]
pub enum DomainPattern {
    ApexAndSubdomains(String),
    SubdomainsOnly(String),
    Exact(String),
}

impl DomainPattern {
    pub fn parse_for_constraints(input: &str) -> Self {
        let input = input.trim();
        if input.is_empty() {
            return Self::Exact(String::new());
        }
        if let Some(domain) = input.strip_prefix("**.") {
            return Self::ApexAndSubdomains(parse_domain_for_constraints(domain));
        }
        if let Some(domain) = input.strip_prefix("*.") {
            return Self::SubdomainsOnly(parse_domain_for_constraints(domain));
        }
        Self::Exact(parse_domain_for_constraints(input))
    }

    pub fn allows(&self, candidate: &DomainPattern) -> bool {
        match self {
            Self::Exact(domain) => match candidate {
                Self::Exact(candidate) => domain_eq(candidate, domain),
                _ => false,
            },
            Self::SubdomainsOnly(domain) => match candidate {
                Self::Exact(candidate) => is_strict_subdomain(candidate, domain),
                Self::SubdomainsOnly(candidate) => is_subdomain_or_equal(candidate, domain),
                Self::ApexAndSubdomains(candidate) => is_strict_subdomain(candidate, domain),
            },
            Self::ApexAndSubdomains(domain) => match candidate {
                Self::Exact(candidate)
                | Self::SubdomainsOnly(candidate)
                | Self::ApexAndSubdomains(candidate) => is_subdomain_or_equal(candidate, domain),
            },
        }
    }
}

fn parse_domain_for_constraints(domain: &str) -> String {
    let domain = domain.trim().trim_end_matches('.');
    if domain.is_empty() {
        return String::new();
    }
    let host = if domain.starts_with('[') && domain.ends_with(']') {
        &domain[1..domain.len().saturating_sub(1)]
    } else {
        domain
    };
    if host.contains('*') || host.contains('?') || host.contains('%') {
        return domain.to_string();
    }
    match UrlHost::parse(host) {
        Ok(host) => host.to_string(),
        Err(_) => String::new(),
    }
}

fn expand_domain_pattern(pattern: &str) -> Vec<String> {
    let input = pattern.trim();
    if let Some(domain) = input.strip_prefix("**.") {
        vec![domain.to_string(), format!("?*.{domain}")]
    } else if let Some(domain) = input.strip_prefix("*.") {
        vec![format!("?*.{domain}")]
    } else {
        vec![input.to_string()]
    }
}

fn normalize_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

fn domain_eq(left: &str, right: &str) -> bool {
    normalize_domain(left) == normalize_domain(right)
}

fn is_subdomain_or_equal(candidate: &str, domain: &str) -> bool {
    domain_eq(candidate, domain) || is_strict_subdomain(candidate, domain)
}

fn is_strict_subdomain(candidate: &str, domain: &str) -> bool {
    let candidate = normalize_domain(candidate);
    let domain = normalize_domain(domain);
    candidate.len() > domain.len()
        && candidate.ends_with(&domain)
        && candidate.as_bytes()[candidate.len() - domain.len() - 1] == b'.'
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkProxyConstraints {
    pub enabled: Option<bool>,
    pub mode: Option<NetworkMode>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    pub domains: Option<NetworkDomainPermissions>,
    pub managed_allowed_domains_only: Option<bool>,
    pub unix_sockets: Option<NetworkUnixSocketPermissions>,
    pub allow_local_binding: Option<bool>,
}

impl NetworkProxyConstraints {
    pub fn allowed_domains(&self) -> Option<Vec<String>> {
        domain_entries(self.domains.as_ref(), NetworkDomainPermission::Allow)
    }

    pub fn denied_domains(&self) -> Option<Vec<String>> {
        domain_entries(self.domains.as_ref(), NetworkDomainPermission::Deny)
    }

    pub fn allow_unix_sockets(&self) -> Vec<String> {
        self.unix_sockets
            .as_ref()
            .map(|unix_sockets| {
                unix_sockets
                    .entries
                    .iter()
                    .filter(|(_, permission)| {
                        matches!(permission, NetworkUnixSocketPermission::Allow)
                    })
                    .map(|(path, _)| path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn domain_entries(
    domains: Option<&NetworkDomainPermissions>,
    permission: NetworkDomainPermission,
) -> Option<Vec<String>> {
    domains
        .map(|domains| {
            domains
                .effective_entries()
                .into_iter()
                .filter(|entry| entry.permission == permission)
                .map(|entry| entry.pattern)
                .collect()
        })
        .filter(|entries: &Vec<String>| !entries.is_empty())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid value for {field_name}: {candidate} (allowed {allowed})")]
pub struct NetworkProxyConstraintError {
    pub field_name: String,
    pub candidate: String,
    pub allowed: String,
}

pub fn validate_unix_socket_allowlist_paths(config: &NetworkProxyConfig) -> Result<()> {
    if config.network.dangerously_allow_all_unix_sockets {
        return Ok(());
    }
    for path in config.network.allow_unix_sockets() {
        if !Path::new(&path).is_absolute() {
            bail!("network.allow_unix_sockets entries must be absolute paths: {path}");
        }
    }
    Ok(())
}

pub fn validate_policy_against_constraints(
    config: &NetworkProxyConfig,
    constraints: &NetworkProxyConstraints,
) -> std::result::Result<(), NetworkProxyConstraintError> {
    validate_unix_socket_allowlist_paths(config).map_err(|err| {
        invalid(
            "network.allow_unix_sockets",
            err.to_string(),
            "absolute paths",
        )
    })?;
    let allowed_domains = config.network.allowed_domains().unwrap_or_default();
    let denied_domains = config.network.denied_domains().unwrap_or_default();
    validate_non_global_wildcard_domain_patterns("network.denied_domains", &denied_domains)?;

    validate_bool_max(
        "network.enabled",
        config.network.enabled,
        constraints.enabled,
    )?;
    validate_bool_max(
        "network.allow_upstream_proxy",
        config.network.allow_upstream_proxy,
        constraints.allow_upstream_proxy,
    )?;
    validate_bool_max(
        "network.dangerously_allow_non_loopback_proxy",
        config.network.dangerously_allow_non_loopback_proxy,
        constraints.dangerously_allow_non_loopback_proxy,
    )?;
    validate_bool_max(
        "network.dangerously_allow_all_unix_sockets",
        config.network.dangerously_allow_all_unix_sockets,
        constraints.dangerously_allow_all_unix_sockets,
    )?;
    validate_bool_max(
        "network.allow_local_binding",
        config.network.allow_local_binding,
        constraints.allow_local_binding,
    )?;
    if let Some(max_mode) = constraints.mode
        && network_mode_rank(config.network.mode) > network_mode_rank(max_mode)
    {
        return Err(invalid(
            "network.mode",
            format!("{:?}", config.network.mode),
            format!("{max_mode:?} or stricter"),
        ));
    }

    if let Some(managed_allowed) = constraints.allowed_domains() {
        validate_non_global_wildcard_domain_patterns("network.allowed_domains", &managed_allowed)?;
        let managed_patterns: Vec<DomainPattern> = managed_allowed
            .iter()
            .map(|entry| DomainPattern::parse_for_constraints(entry))
            .collect();
        for candidate in &allowed_domains {
            let candidate_pattern = DomainPattern::parse_for_constraints(candidate);
            if !managed_patterns
                .iter()
                .any(|managed| managed.allows(&candidate_pattern))
            {
                return Err(invalid(
                    "network.allowed_domains",
                    candidate.clone(),
                    "subset of managed allowed_domains",
                ));
            }
        }
        if constraints.managed_allowed_domains_only.unwrap_or(false) {
            let required: HashSet<String> = managed_allowed
                .iter()
                .map(|entry| normalize_pattern(entry))
                .collect();
            let candidate: HashSet<String> = allowed_domains
                .iter()
                .map(|entry| normalize_pattern(entry))
                .collect();
            if candidate != required {
                return Err(invalid(
                    "network.allowed_domains",
                    format!("{allowed_domains:?}"),
                    "must match managed allowed_domains",
                ));
            }
        }
    }

    if let Some(managed_denied) = constraints.denied_domains() {
        validate_non_global_wildcard_domain_patterns("network.denied_domains", &managed_denied)?;
        let required: HashSet<String> = managed_denied
            .iter()
            .map(|entry| normalize_pattern(entry))
            .collect();
        let candidate: HashSet<String> = denied_domains
            .iter()
            .map(|entry| normalize_pattern(entry))
            .collect();
        if !required.is_subset(&candidate) {
            return Err(invalid(
                "network.denied_domains",
                format!("{denied_domains:?}"),
                "missing managed denied_domains entries",
            ));
        }
    }

    let managed_unix_sockets = constraints.allow_unix_sockets();
    if !managed_unix_sockets.is_empty() {
        let allowed: HashSet<String> = managed_unix_sockets
            .iter()
            .map(|entry| entry.to_ascii_lowercase())
            .collect();
        for candidate in config.network.allow_unix_sockets() {
            if !allowed.contains(&candidate.to_ascii_lowercase()) {
                return Err(invalid(
                    "network.allow_unix_sockets",
                    candidate,
                    "subset of managed allow_unix_sockets",
                ));
            }
        }
    }

    Ok(())
}

fn validate_bool_max(
    field_name: &'static str,
    candidate: bool,
    max: Option<bool>,
) -> std::result::Result<(), NetworkProxyConstraintError> {
    if candidate && max == Some(false) {
        Err(invalid(field_name, "true", "false"))
    } else {
        Ok(())
    }
}

fn validate_non_global_wildcard_domain_patterns(
    field_name: &'static str,
    patterns: &[String],
) -> std::result::Result<(), NetworkProxyConstraintError> {
    for pattern in patterns {
        if is_global_wildcard_domain_pattern(pattern) {
            return Err(invalid(
                field_name,
                pattern.clone(),
                "exact hosts or scoped wildcards like *.example.com or **.example.com",
            ));
        }
    }
    Ok(())
}

fn invalid(
    field_name: impl Into<String>,
    candidate: impl Into<String>,
    allowed: impl Into<String>,
) -> NetworkProxyConstraintError {
    NetworkProxyConstraintError {
        field_name: field_name.into(),
        candidate: candidate.into(),
        allowed: allowed.into(),
    }
}

fn network_mode_rank(mode: NetworkMode) -> u8 {
    match mode {
        NetworkMode::Limited => 0,
        NetworkMode::Full => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkPolicyDecision {
    Allow,
    Deny { reason: String },
}

#[derive(Debug, Clone)]
pub struct NetworkProxyState {
    config: NetworkProxyConfig,
    allow_set: GlobSet,
    deny_set: GlobSet,
}

impl NetworkProxyState {
    pub fn new(config: NetworkProxyConfig) -> Result<Self> {
        validate_unix_socket_allowlist_paths(&config)?;
        Ok(Self {
            allow_set: compile_allowlist_globset(
                &config.network.allowed_domains().unwrap_or_default(),
            )?,
            deny_set: compile_denylist_globset(
                &config.network.denied_domains().unwrap_or_default(),
            )?,
            config,
        })
    }

    pub fn config(&self) -> &NetworkProxyConfig {
        &self.config
    }

    pub fn decide_host(&self, host: &str, method: Option<&str>) -> NetworkPolicyDecision {
        if let Some(method) = method
            && !self.config.network.mode.allows_method(method)
        {
            return NetworkPolicyDecision::Deny {
                reason: "blocked-by-method-policy".to_string(),
            };
        }
        let host = normalize_host(host);
        if self.deny_set.is_match(&host) {
            return NetworkPolicyDecision::Deny {
                reason: "blocked-by-denylist".to_string(),
            };
        }
        if self.allow_set.is_empty() || self.allow_set.is_match(&host) {
            return NetworkPolicyDecision::Allow;
        }
        NetworkPolicyDecision::Deny {
            reason: "blocked-by-allowlist".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_domain_permission_normalizes_and_replaces_existing_entry() {
        let mut config = NetworkConfig::default();

        config.upsert_domain_permission(
            " EXAMPLE.COM ".to_string(),
            NetworkDomainPermission::Allow,
            normalize_host,
        );
        config.upsert_domain_permission(
            "example.com".to_string(),
            NetworkDomainPermission::Deny,
            normalize_host,
        );

        let domains = config.domains.expect("domains");
        assert_eq!(
            domains.entries,
            vec![NetworkDomainPermissionEntry {
                pattern: "example.com".to_string(),
                permission: NetworkDomainPermission::Deny,
            }]
        );
    }

    #[test]
    fn normalize_host_matches_proxy_policy_cases() {
        assert_eq!(normalize_host("  ExAmPlE.CoM  "), "example.com");
        assert_eq!(normalize_host("example.com:443"), "example.com");
        assert_eq!(normalize_host("example.com."), "example.com");
        assert_eq!(normalize_host("[::1]:443"), "::1");
        assert_eq!(normalize_host("[fe80::1%25lo0]"), "fe80::1%lo0");
    }

    #[test]
    fn limited_mode_allows_only_safe_methods() {
        assert!(NetworkMode::Limited.allows_method("GET"));
        assert!(NetworkMode::Limited.allows_method("HEAD"));
        assert!(!NetworkMode::Limited.allows_method("POST"));
        assert!(!NetworkMode::Limited.allows_method("CONNECT"));
    }

    #[test]
    fn globsets_match_apex_and_subdomains() {
        let set = compile_allowlist_globset(&["**.Example.COM.".to_string()]).unwrap();
        assert!(set.is_match("example.com"));
        assert!(set.is_match("api.example.com"));
        assert!(!set.is_match("other.com"));
    }

    #[test]
    fn denylist_rejects_global_wildcard() {
        let err = compile_denylist_globset(&["*".to_string()]).expect_err("reject wildcard");
        assert!(err.to_string().contains("unsupported global wildcard"));
    }

    #[test]
    fn validates_allowed_domains_subset() {
        let mut config = NetworkProxyConfig::default();
        config.network.upsert_domain_permission(
            "api.example.com".to_string(),
            NetworkDomainPermission::Allow,
            normalize_host,
        );
        let constraints = NetworkProxyConstraints {
            domains: Some(NetworkDomainPermissions {
                entries: vec![NetworkDomainPermissionEntry {
                    pattern: "**.example.com".to_string(),
                    permission: NetworkDomainPermission::Allow,
                }],
            }),
            ..Default::default()
        };

        validate_policy_against_constraints(&config, &constraints).unwrap();
    }

    #[test]
    fn state_decides_allow_deny_and_method_policy() {
        let mut config = NetworkProxyConfig::default();
        config.network.mode = NetworkMode::Limited;
        config.network.upsert_domain_permission(
            "**.example.com".to_string(),
            NetworkDomainPermission::Allow,
            normalize_host,
        );
        config.network.upsert_domain_permission(
            "blocked.example.com".to_string(),
            NetworkDomainPermission::Deny,
            normalize_host,
        );
        let state = NetworkProxyState::new(config).unwrap();

        assert_eq!(
            state.decide_host("api.example.com", Some("GET")),
            NetworkPolicyDecision::Allow
        );
        assert_eq!(
            state.decide_host("api.example.com", Some("POST")),
            NetworkPolicyDecision::Deny {
                reason: "blocked-by-method-policy".to_string()
            }
        );
        assert_eq!(
            state.decide_host("blocked.example.com", Some("GET")),
            NetworkPolicyDecision::Deny {
                reason: "blocked-by-denylist".to_string()
            }
        );
    }
}
