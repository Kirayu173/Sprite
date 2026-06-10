use super::*;
use pretty_assertions::assert_eq;
use runtime_protocol::config_types::ModelProviderAuthInfo;
use std::num::NonZeroU64;
use tempfile::tempdir;
use utils_absolute_path::AbsolutePathBuf;
use utils_absolute_path::AbsolutePathBufGuard;

#[test]
fn deserialize_local_provider_defaults_to_responses_wire_api() {
    let provider: ModelProviderInfo = toml::from_str(
        r#"
name = "Ollama"
base_url = "http://localhost:11434/v1"
"#,
    )
    .expect("provider");

    assert_eq!(provider.name, "Ollama");
    assert_eq!(
        provider.base_url.as_deref(),
        Some("http://localhost:11434/v1")
    );
    assert_eq!(provider.wire_api, WireApi::Responses);
}

#[test]
fn deserialize_chat_wire_api_shows_helpful_error() {
    let provider_toml = r#"
name = "OpenAI using Chat Completions"
base_url = "https://provider.example/v1"
wire_api = "chat"
"#;

    let err = toml::from_str::<ModelProviderInfo>(provider_toml).expect_err("chat unsupported");
    assert!(err.to_string().contains(CHAT_WIRE_API_REMOVED_ERROR));
}

#[test]
fn deserialize_anthropic_compatible_wire_api() {
    let provider: ModelProviderInfo = toml::from_str(
        r#"
name = "Anthropic-compatible"
base_url = "https://example.com/anthropic"
wire_api = "anthropic-compatible"
"#,
    )
    .expect("provider");

    assert_eq!(provider.wire_api, WireApi::AnthropicCompatible);
}

#[test]
fn create_anthropic_compatible_provider() {
    assert_eq!(
        ModelProviderInfo::create_anthropic_compatible_provider(Some(
            "https://example.com/anthropic".to_string(),
        )),
        ModelProviderInfo {
            name: "Anthropic-compatible".to_string(),
            base_url: Some("https://example.com/anthropic".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            wire_api: WireApi::AnthropicCompatible,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            supports_websockets: false,
        }
    );
}

#[test]
fn deserialize_provider_auth_config_defaults() {
    let base_dir = tempdir().expect("tempdir");
    let provider: ModelProviderInfo = {
        let _guard = AbsolutePathBufGuard::new(base_dir.path());
        toml::from_str(
            r#"
name = "Corp"

[auth]
command = "./scripts/print-token"
args = ["--format=text"]
"#,
        )
        .expect("provider")
    };

    assert_eq!(
        provider.auth,
        Some(ModelProviderAuthInfo {
            command: "./scripts/print-token".to_string(),
            args: vec!["--format=text".to_string()],
            timeout_ms: NonZeroU64::new(5_000).expect("non-zero timeout"),
            refresh_interval_ms: 300_000,
            cwd: AbsolutePathBuf::resolve_path_against_base(".", base_dir.path()),
        })
    );
}

#[test]
fn built_in_local_providers_build_api_providers_without_user_config() {
    let providers = built_in_model_providers(None);

    for id in [OLLAMA_OSS_PROVIDER_ID, LMSTUDIO_OSS_PROVIDER_ID] {
        providers[id]
            .to_api_provider()
            .unwrap_or_else(|err| panic!("provider `{id}` should be usable: {err}"));
    }

    assert_eq!(providers[OLLAMA_OSS_PROVIDER_ID].name, "Ollama");
    assert_eq!(providers[LMSTUDIO_OSS_PROVIDER_ID].name, "LM Studio");
    assert_eq!(
        providers[OPENAI_COMPATIBLE_PROVIDER_ID]
            .to_api_provider()
            .expect_err("base_url required")
            .to_string(),
        "OpenAI-compatible: OpenAI-compatible providers must set `base_url` explicitly"
    );
}

#[test]
fn default_model_for_provider_uses_provider_specific_defaults() {
    assert_eq!(
        default_model_for_provider(OLLAMA_OSS_PROVIDER_ID),
        DEFAULT_OLLAMA_MODEL
    );
    assert_eq!(
        default_model_for_provider(LMSTUDIO_OSS_PROVIDER_ID),
        DEFAULT_LMSTUDIO_MODEL
    );
    assert_eq!(
        default_model_for_provider(OPENAI_COMPATIBLE_PROVIDER_ID),
        DEFAULT_OPENAI_COMPATIBLE_MODEL
    );
}

#[test]
fn merge_configured_model_providers_adds_custom_provider() {
    let custom_provider = ModelProviderInfo {
        name: "Custom".to_string(),
        base_url: Some("https://example.com/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    let configured =
        std::collections::HashMap::from([("custom".to_string(), custom_provider.clone())]);

    let mut expected = built_in_model_providers(None);
    expected.insert("custom".to_string(), custom_provider);

    assert_eq!(
        merge_configured_model_providers(built_in_model_providers(None), configured),
        Ok(expected)
    );
}

#[test]
fn merge_configured_model_providers_preserves_built_in_provider() {
    let configured = std::collections::HashMap::from([(
        OLLAMA_OSS_PROVIDER_ID.to_string(),
        ModelProviderInfo {
            name: "Custom Ollama".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            ..ModelProviderInfo::default()
        },
    )]);

    assert_eq!(
        merge_configured_model_providers(built_in_model_providers(None), configured),
        Ok(built_in_model_providers(None))
    );
}

#[test]
fn validate_provider_auth_rejects_conflicting_env_key() {
    let provider = ModelProviderInfo {
        env_key: Some("SPRITE_TOKEN".to_string()),
        auth: Some(ModelProviderAuthInfo {
            command: "print-token".to_string(),
            args: Vec::new(),
            timeout_ms: NonZeroU64::new(5_000).expect("timeout"),
            refresh_interval_ms: 300_000,
            cwd: AbsolutePathBuf::resolve_path_against_base(
                ".",
                std::env::current_dir().expect("cwd"),
            ),
        }),
        ..ModelProviderInfo::create_openai_compatible_provider(Some(
            "https://provider.example/v1".to_string(),
        ))
    };

    assert_eq!(
        provider.validate(),
        Err("provider auth cannot be combined with env_key".to_string())
    );
}

#[test]
fn deserialize_provider_auth_config_allows_zero_refresh_interval() {
    let base_dir = tempdir().expect("tempdir");
    let provider: ModelProviderInfo = {
        let _guard = AbsolutePathBufGuard::new(base_dir.path());
        toml::from_str(
            r#"
name = "Corp"

[auth]
command = "./scripts/print-token"
refresh_interval_ms = 0
"#,
        )
        .expect("provider")
    };

    let auth = provider.auth.expect("auth");
    assert_eq!(auth.refresh_interval_ms, 0);
    assert_eq!(auth.refresh_interval(), None);
}
