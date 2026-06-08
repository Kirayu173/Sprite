use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use runtime_protocol::config_types::TrustLevel;
use tokio::task;
use toml::Value as TomlValue;
use toml_edit::DocumentMut;
use toml_edit::Item as TomlItem;
use toml_edit::Table as TomlTable;
use toml_edit::value;
use utils_path::resolve_symlink_write_paths;
use utils_path::write_atomically;

use crate::AppToolApproval;
use crate::CONFIG_TOML_FILE;
use crate::McpServerConfig;
use crate::McpServerEnvVar;
use crate::McpServerTransportConfig;

pub async fn load_global_mcp_servers(
    sprite_home: &Path,
) -> std::io::Result<BTreeMap<String, McpServerConfig>> {
    let config_path = sprite_home.join(CONFIG_TOML_FILE);
    let raw = match tokio::fs::read_to_string(&config_path).await {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => return Err(err),
    };
    let parsed = toml::from_str::<TomlValue>(&raw)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))?;
    let Some(servers_value) = parsed.get("mcp_servers") else {
        return Ok(BTreeMap::new());
    };

    ensure_no_inline_bearer_tokens(servers_value)?;

    servers_value
        .clone()
        .try_into()
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))
}

fn ensure_no_inline_bearer_tokens(value: &TomlValue) -> std::io::Result<()> {
    let Some(servers_table) = value.as_table() else {
        return Ok(());
    };

    for (server_name, server_value) in servers_table {
        if let Some(server_table) = server_value.as_table()
            && server_table.contains_key("bearer_token")
        {
            let message = format!(
                "mcp_servers.{server_name} uses unsupported `bearer_token`; set `bearer_token_env_var`."
            );
            return Err(std::io::Error::new(ErrorKind::InvalidData, message));
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub enum ConfigEdit {
    ReplaceMcpServers(BTreeMap<String, McpServerConfig>),
    SetPath {
        segments: Vec<String>,
        value: TomlItem,
    },
    ClearPath {
        segments: Vec<String>,
    },
    SetProjectTrustLevel {
        path: PathBuf,
        level: TrustLevel,
    },
    SetPluginEnabled {
        plugin_key: String,
        enabled: bool,
    },
    ClearPlugin {
        plugin_key: String,
    },
}

pub struct ConfigEditsBuilder {
    config_path: PathBuf,
    edits: Vec<ConfigEdit>,
}

impl ConfigEditsBuilder {
    pub fn new(sprite_home: &Path) -> Self {
        Self::for_config_path(&sprite_home.join(CONFIG_TOML_FILE))
    }

    pub fn for_config_path(config_path: &Path) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
            edits: Vec::new(),
        }
    }

    pub fn replace_mcp_servers(mut self, servers: &BTreeMap<String, McpServerConfig>) -> Self {
        self.edits
            .push(ConfigEdit::ReplaceMcpServers(servers.clone()));
        self
    }

    pub fn set_path(mut self, segments: Vec<String>, value: TomlItem) -> Self {
        self.edits.push(ConfigEdit::SetPath { segments, value });
        self
    }

    pub fn clear_path(mut self, segments: Vec<String>) -> Self {
        self.edits.push(ConfigEdit::ClearPath { segments });
        self
    }

    pub fn set_project_trust_level<P: Into<PathBuf>>(mut self, path: P, level: TrustLevel) -> Self {
        self.edits.push(ConfigEdit::SetProjectTrustLevel {
            path: path.into(),
            level,
        });
        self
    }

    pub fn set_plugin_enabled(mut self, plugin_key: String, enabled: bool) -> Self {
        self.edits.push(ConfigEdit::SetPluginEnabled {
            plugin_key,
            enabled,
        });
        self
    }

    pub fn clear_plugin(mut self, plugin_key: String) -> Self {
        self.edits.push(ConfigEdit::ClearPlugin { plugin_key });
        self
    }

    pub async fn apply(self) -> std::io::Result<()> {
        task::spawn_blocking(move || self.apply_blocking())
            .await
            .map_err(|err| {
                std::io::Error::other(format!("config persistence task panicked: {err}"))
            })?
    }

    pub fn apply_blocking(self) -> std::io::Result<()> {
        if self.edits.is_empty() {
            return Ok(());
        }
        let write_paths = resolve_symlink_write_paths(&self.config_path)?;
        let mut doc = read_or_create_document(write_paths.read_path.as_deref())?;
        let mut mutated = false;
        for edit in self.edits {
            mutated |= apply_edit(&mut doc, edit);
        }
        if !mutated {
            return Ok(());
        }
        write_atomically(&write_paths.write_path, &doc.to_string())
    }
}

pub async fn set_user_plugin_enabled(
    sprite_home: &Path,
    plugin_key: String,
    enabled: bool,
) -> std::io::Result<()> {
    ConfigEditsBuilder::new(sprite_home)
        .set_plugin_enabled(plugin_key, enabled)
        .apply()
        .await
}

pub async fn clear_user_plugin(sprite_home: &Path, plugin_key: String) -> std::io::Result<()> {
    ConfigEditsBuilder::new(sprite_home)
        .clear_plugin(plugin_key)
        .apply()
        .await
}

fn read_or_create_document(config_path: Option<&Path>) -> std::io::Result<DocumentMut> {
    let Some(config_path) = config_path else {
        return Ok(DocumentMut::new());
    };
    match fs::read_to_string(config_path) {
        Ok(raw) => raw
            .parse::<DocumentMut>()
            .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(err) => Err(err),
    }
}

fn apply_edit(doc: &mut DocumentMut, edit: ConfigEdit) -> bool {
    match edit {
        ConfigEdit::ReplaceMcpServers(servers) => replace_mcp_servers(doc, &servers),
        ConfigEdit::SetPath { segments, value } => insert_path(doc, &segments, value),
        ConfigEdit::ClearPath { segments } => remove_path(doc, &segments),
        ConfigEdit::SetProjectTrustLevel { path, level } => {
            set_project_trust_level(doc, &path, level)
        }
        ConfigEdit::SetPluginEnabled {
            plugin_key,
            enabled,
        } => set_plugin_enabled(doc, &plugin_key, enabled),
        ConfigEdit::ClearPlugin { plugin_key } => clear_plugin(doc, &plugin_key),
    }
}

fn replace_mcp_servers(doc: &mut DocumentMut, servers: &BTreeMap<String, McpServerConfig>) -> bool {
    let root = doc.as_table_mut();
    if servers.is_empty() {
        return root.remove("mcp_servers").is_some();
    }

    let mut table = TomlTable::new();
    table.set_implicit(true);
    for (name, config) in servers {
        table.insert(name, serialize_mcp_server(config));
    }
    root.insert("mcp_servers", TomlItem::Table(table));
    true
}

fn serialize_mcp_server(config: &McpServerConfig) -> TomlItem {
    let mut entry = TomlTable::new();
    entry.set_implicit(false);

    match &config.transport {
        McpServerTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
        } => {
            entry["command"] = value(command.clone());
            if !args.is_empty() {
                entry["args"] = array_from_strings(args);
            }
            if let Some(env) = env
                && !env.is_empty()
            {
                entry["env"] = table_from_pairs(env.iter());
            }
            if !env_vars.is_empty() {
                entry["env_vars"] = array_from_env_vars(env_vars);
            }
            if let Some(cwd) = cwd {
                entry["cwd"] = value(cwd.to_string_lossy().to_string());
            }
        }
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            entry["url"] = value(url.clone());
            if let Some(env_var) = bearer_token_env_var {
                entry["bearer_token_env_var"] = value(env_var.clone());
            }
            if let Some(headers) = http_headers
                && !headers.is_empty()
            {
                entry["http_headers"] = table_from_pairs(headers.iter());
            }
            if let Some(headers) = env_http_headers
                && !headers.is_empty()
            {
                entry["env_http_headers"] = table_from_pairs(headers.iter());
            }
        }
    }

    if !config.enabled {
        entry["enabled"] = value(false);
    }
    if !config.is_local_environment() {
        entry["environment_id"] = value(config.environment_id.clone());
    }
    if config.required {
        entry["required"] = value(true);
    }
    if config.supports_parallel_tool_calls {
        entry["supports_parallel_tool_calls"] = value(true);
    }
    if let Some(timeout) = config.startup_timeout_sec {
        entry["startup_timeout_sec"] = value(timeout.as_secs_f64());
    }
    if let Some(timeout) = config.tool_timeout_sec {
        entry["tool_timeout_sec"] = value(timeout.as_secs_f64());
    }
    if let Some(approval_mode) = config.default_tools_approval_mode {
        entry["default_tools_approval_mode"] = value(match approval_mode {
            AppToolApproval::Auto => "auto",
            AppToolApproval::Prompt => "prompt",
            AppToolApproval::Approve => "approve",
        });
    }
    if let Some(enabled_tools) = &config.enabled_tools
        && !enabled_tools.is_empty()
    {
        entry["enabled_tools"] = array_from_strings(enabled_tools);
    }
    if let Some(disabled_tools) = &config.disabled_tools
        && !disabled_tools.is_empty()
    {
        entry["disabled_tools"] = array_from_strings(disabled_tools);
    }
    if let Some(scopes) = &config.scopes
        && !scopes.is_empty()
    {
        entry["scopes"] = array_from_strings(scopes);
    }
    if let Some(oauth) = &config.oauth
        && let Some(client_id) = &oauth.client_id
        && !client_id.is_empty()
    {
        let mut oauth_table = TomlTable::new();
        oauth_table.set_implicit(false);
        oauth_table["client_id"] = value(client_id.clone());
        entry["oauth"] = TomlItem::Table(oauth_table);
    }
    if let Some(resource) = &config.oauth_resource
        && !resource.is_empty()
    {
        entry["oauth_resource"] = value(resource.clone());
    }
    if !config.tools.is_empty() {
        let mut tools = TomlTable::new();
        tools.set_implicit(false);
        let mut tool_entries: Vec<_> = config.tools.iter().collect();
        tool_entries.sort_by_key(|(name, _)| *name);
        for (name, tool_config) in tool_entries {
            let mut tool_entry = TomlTable::new();
            tool_entry.set_implicit(false);
            if let Some(approval_mode) = tool_config.approval_mode {
                tool_entry["approval_mode"] = value(match approval_mode {
                    AppToolApproval::Auto => "auto",
                    AppToolApproval::Prompt => "prompt",
                    AppToolApproval::Approve => "approve",
                });
            }
            tools.insert(name, TomlItem::Table(tool_entry));
        }
        entry.insert("tools", TomlItem::Table(tools));
    }

    TomlItem::Table(entry)
}

fn array_from_strings(values: &[String]) -> TomlItem {
    let mut array = toml_edit::Array::new();
    for value in values {
        array.push(value.clone());
    }
    TomlItem::Value(array.into())
}

fn array_from_env_vars(env_vars: &[McpServerEnvVar]) -> TomlItem {
    let mut array = toml_edit::Array::new();
    for env_var in env_vars {
        match env_var {
            McpServerEnvVar::Name(name) => array.push(name.clone()),
            McpServerEnvVar::Config { name, source } => {
                let mut table = toml_edit::InlineTable::new();
                table.insert("name", name.clone().into());
                if let Some(source) = source {
                    table.insert("source", source.clone().into());
                }
                array.push(table);
            }
        }
    }
    TomlItem::Value(array.into())
}

fn table_from_pairs<'a, I>(pairs: I) -> TomlItem
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut entries: Vec<_> = pairs.into_iter().collect();
    entries.sort_by_key(|(key, _)| *key);
    let mut table = TomlTable::new();
    table.set_implicit(false);
    for (key, value_str) in entries {
        table.insert(key, value(value_str.clone()));
    }
    TomlItem::Table(table)
}

fn insert_path(doc: &mut DocumentMut, segments: &[String], value: TomlItem) -> bool {
    let Some((last, parents)) = segments.split_last() else {
        return false;
    };
    let Some(parent) = descend(doc, parents, TraversalMode::Create) else {
        return false;
    };
    parent[last] = value;
    true
}

fn remove_path(doc: &mut DocumentMut, segments: &[String]) -> bool {
    let Some((last, parents)) = segments.split_last() else {
        return false;
    };
    let Some(parent) = descend(doc, parents, TraversalMode::Existing) else {
        return false;
    };
    parent.remove(last).is_some()
}

fn set_project_trust_level(doc: &mut DocumentMut, path: &Path, level: TrustLevel) -> bool {
    let segments = vec![
        "projects".to_string(),
        path.to_string_lossy().to_string(),
        "trust_level".to_string(),
    ];
    insert_path(doc, &segments, value(level.to_string()))
}

fn set_plugin_enabled(doc: &mut DocumentMut, plugin_key: &str, enabled: bool) -> bool {
    let segments = vec![
        "plugins".to_string(),
        plugin_key.to_string(),
        "enabled".to_string(),
    ];
    insert_path(doc, &segments, value(enabled))
}

fn clear_plugin(doc: &mut DocumentMut, plugin_key: &str) -> bool {
    let segments = vec!["plugins".to_string(), plugin_key.to_string()];
    let removed = remove_path(doc, &segments);
    if doc
        .get("plugins")
        .and_then(TomlItem::as_table_like)
        .is_some_and(|plugins| plugins.is_empty())
    {
        doc.as_table_mut().remove("plugins");
    }
    removed
}

#[derive(Clone, Copy)]
enum TraversalMode {
    Create,
    Existing,
}

fn descend<'a>(
    doc: &'a mut DocumentMut,
    segments: &[String],
    mode: TraversalMode,
) -> Option<&'a mut TomlTable> {
    let mut current = doc.as_table_mut();

    for segment in segments {
        match mode {
            TraversalMode::Create => {
                if !current.contains_key(segment.as_str()) {
                    current.insert(segment.as_str(), TomlItem::Table(new_implicit_table()));
                }
                let item = current.get_mut(segment.as_str())?;
                current = ensure_table_for_write(item)?;
            }
            TraversalMode::Existing => {
                let item = current.get_mut(segment.as_str())?;
                current = ensure_table_for_read(item)?;
            }
        }
    }

    Some(current)
}

fn ensure_table_for_write(item: &mut TomlItem) -> Option<&mut TomlTable> {
    match item {
        TomlItem::Table(table) => Some(table),
        TomlItem::Value(value) => {
            let table = value
                .as_inline_table()
                .map_or_else(new_implicit_table, table_from_inline);
            *item = TomlItem::Table(table);
            item.as_table_mut()
        }
        TomlItem::None => {
            *item = TomlItem::Table(new_implicit_table());
            item.as_table_mut()
        }
        _ => None,
    }
}

fn ensure_table_for_read(item: &mut TomlItem) -> Option<&mut TomlTable> {
    match item {
        TomlItem::Table(_) => {}
        TomlItem::Value(value) => {
            let inline = value.as_inline_table()?.clone();
            *item = TomlItem::Table(table_from_inline(&inline));
        }
        _ => return None,
    }
    item.as_table_mut()
}

fn table_from_inline(inline: &toml_edit::InlineTable) -> TomlTable {
    let mut table = new_implicit_table();
    for (key, value) in inline.iter() {
        let mut value = value.clone();
        value.decor_mut().set_suffix("");
        table.insert(key, TomlItem::Value(value));
    }
    table
}

fn new_implicit_table() -> TomlTable {
    let mut table = TomlTable::new();
    table.set_implicit(true);
    table
}

#[cfg(test)]
#[path = "mcp_edit_tests.rs"]
mod tests;
