use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    Artifact,
    CodeMode,
    NetworkProxy,
}

#[derive(Debug, Clone, Copy)]
pub struct FeatureInfo {
    pub id: Feature,
    pub key: &'static str,
}

pub const FEATURES: &[FeatureInfo] = &[
    FeatureInfo {
        id: Feature::Artifact,
        key: "artifact",
    },
    FeatureInfo {
        id: Feature::CodeMode,
        key: "code_mode",
    },
    FeatureInfo {
        id: Feature::NetworkProxy,
        key: "network_proxy",
    },
];

pub fn legacy_feature_keys() -> Vec<&'static str> {
    Vec::new()
}

pub fn is_known_feature_key(key: &str) -> bool {
    FEATURES.iter().any(|feature| feature.key == key)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FeatureToml<T = serde_json::Value> {
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub config: Option<T>,
}

pub type FeaturesToml = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodeModeConfigToml {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkProxyConfigToml {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureOverrides;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureConfigSource {
    Config,
    Managed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Features;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_feature_keys_are_explicit_phase1_keys() {
        assert!(is_known_feature_key("artifact"));
        assert!(is_known_feature_key("code_mode"));
        assert!(is_known_feature_key("network_proxy"));
        assert!(!is_known_feature_key("app_mcp_path_override"));
        assert!(!is_known_feature_key("multi_agent"));
    }

    #[test]
    fn feature_toml_accepts_enabled_and_config_payload() {
        let parsed: FeatureToml<NetworkProxyConfigToml> =
            serde_json::from_value(serde_json::json!({ "enabled": true })).expect("feature config");

        assert_eq!(parsed.enabled, Some(true));
        assert_eq!(parsed.config, Some(NetworkProxyConfigToml {}));
    }
}
