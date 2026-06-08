use super::*;
use crate::McpServerOAuthConfig;
use crate::McpServerToolConfig;
use pretty_assertions::assert_eq;
use runtime_protocol::config_types::TrustLevel;
use std::collections::HashMap;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use toml_edit::value;

#[tokio::test]
async fn replace_mcp_servers_serializes_per_tool_approval_overrides() -> anyhow::Result<()> {
    let unique_suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sprite_home = std::env::temp_dir().join(format!(
        "sprite-config-mcp-edit-test-{}-{unique_suffix}",
        std::process::id()
    ));
    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            environment_id: crate::DEFAULT_MCP_SERVER_ENVIRONMENT_ID.to_string(),
            enabled: true,
            required: false,
            supports_parallel_tool_calls: true,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: Some(AppToolApproval::Auto),
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::from([
                (
                    "search".to_string(),
                    McpServerToolConfig {
                        approval_mode: Some(AppToolApproval::Approve),
                    },
                ),
                (
                    "read".to_string(),
                    McpServerToolConfig {
                        approval_mode: Some(AppToolApproval::Prompt),
                    },
                ),
            ]),
        },
    )]);

    ConfigEditsBuilder::new(&sprite_home)
        .replace_mcp_servers(&servers)
        .apply()
        .await?;

    let config_path = sprite_home.join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert_eq!(
        serialized,
        r#"[mcp_servers.docs]
command = "docs-server"
supports_parallel_tool_calls = true
default_tools_approval_mode = "auto"

[mcp_servers.docs.tools]

[mcp_servers.docs.tools.read]
approval_mode = "prompt"

[mcp_servers.docs.tools.search]
approval_mode = "approve"
"#
    );

    let loaded = load_global_mcp_servers(&sprite_home).await?;
    assert_eq!(loaded, servers);

    std::fs::remove_dir_all(&sprite_home)?;

    Ok(())
}

#[tokio::test]
async fn general_edits_write_and_clear_dotted_paths() -> anyhow::Result<()> {
    let unique_suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sprite_home = std::env::temp_dir().join(format!(
        "sprite-config-general-edit-test-{}-{unique_suffix}",
        std::process::id()
    ));

    ConfigEditsBuilder::new(&sprite_home)
        .set_path(
            vec!["tui".to_string(), "theme".to_string()],
            value("solarized-dark"),
        )
        .set_path(vec!["model".to_string()], value("gpt-5"))
        .clear_path(vec!["model".to_string()])
        .apply()
        .await?;

    let serialized = std::fs::read_to_string(sprite_home.join(CONFIG_TOML_FILE))?;
    assert_eq!(
        serialized,
        r#"[tui]
theme = "solarized-dark"
"#
    );

    std::fs::remove_dir_all(&sprite_home)?;
    Ok(())
}

#[tokio::test]
async fn project_trust_edit_writes_project_table() -> anyhow::Result<()> {
    let unique_suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sprite_home = std::env::temp_dir().join(format!(
        "sprite-config-project-trust-edit-test-{}-{unique_suffix}",
        std::process::id()
    ));
    let project_path = if cfg!(windows) {
        std::path::PathBuf::from(r"C:\work\repo")
    } else {
        std::path::PathBuf::from("/work/repo")
    };

    ConfigEditsBuilder::new(&sprite_home)
        .set_project_trust_level(project_path.clone(), TrustLevel::Trusted)
        .apply()
        .await?;

    let serialized = std::fs::read_to_string(sprite_home.join(CONFIG_TOML_FILE))?;
    assert!(serialized.contains("[projects."));
    assert!(serialized.contains("trust_level = \"trusted\""));

    std::fs::remove_dir_all(&sprite_home)?;
    Ok(())
}

#[tokio::test]
async fn plugin_edit_writes_compiled_plugin_config() -> anyhow::Result<()> {
    let unique_suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sprite_home = std::env::temp_dir().join(format!(
        "sprite-config-plugin-edit-test-{}-{unique_suffix}",
        std::process::id()
    ));

    set_user_plugin_enabled(&sprite_home, "demo@local".to_string(), false).await?;

    let serialized = std::fs::read_to_string(sprite_home.join(CONFIG_TOML_FILE))?;
    assert_eq!(
        serialized,
        r#"[plugins."demo@local"]
enabled = false
"#
    );
    let parsed: crate::config_toml::ConfigToml = toml::from_str(&serialized)?;
    assert_eq!(
        parsed
            .plugins
            .get("demo@local")
            .map(|plugin| plugin.enabled),
        Some(false)
    );

    clear_user_plugin(&sprite_home, "demo@local".to_string()).await?;
    assert_eq!(
        std::fs::read_to_string(sprite_home.join(CONFIG_TOML_FILE))?,
        ""
    );

    std::fs::remove_dir_all(&sprite_home)?;
    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_oauth_client_id() -> anyhow::Result<()> {
    let unique_suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let sprite_home = std::env::temp_dir().join(format!(
        "sprite-config-mcp-oauth-edit-test-{}-{unique_suffix}",
        std::process::id()
    ));
    let servers = BTreeMap::from([(
        "maas_outlook".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            },
            environment_id: crate::DEFAULT_MCP_SERVER_ENVIRONMENT_ID.to_string(),
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: Some(McpServerOAuthConfig {
                client_id: Some("eci-prd-pub-sprite-123".to_string()),
            }),
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    ConfigEditsBuilder::new(&sprite_home)
        .replace_mcp_servers(&servers)
        .apply()
        .await?;

    let config_path = sprite_home.join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert_eq!(
        serialized,
        r#"[mcp_servers.maas_outlook]
url = "https://example.com/mcp"

[mcp_servers.maas_outlook.oauth]
client_id = "eci-prd-pub-sprite-123"
"#
    );

    let loaded = load_global_mcp_servers(&sprite_home).await?;
    assert_eq!(loaded, servers);

    std::fs::remove_dir_all(&sprite_home)?;

    Ok(())
}
