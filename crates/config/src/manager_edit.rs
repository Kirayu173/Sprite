use std::io;
use std::path::Path;

use app_protocol::ConfigWriteErrorCode;
use app_protocol::MergeStrategy;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;
use toml_edit::DocumentMut;
use toml_edit::Item as TomlItem;
use toml_edit::Table as TomlTable;
use utils_json_to_toml::json_to_toml;

use crate::ConfigManagerError;

pub(crate) fn read_or_create_document(
    config_path: Option<&Path>,
) -> Result<DocumentMut, ConfigManagerError> {
    let Some(config_path) = config_path else {
        return Ok(DocumentMut::new());
    };
    match std::fs::read_to_string(config_path) {
        Ok(raw) => raw.parse::<DocumentMut>().map_err(|err| {
            ConfigManagerError::new(
                ConfigWriteErrorCode::ConfigValidationError,
                format!("failed to parse config TOML: {err}"),
            )
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(err) => Err(ConfigManagerError::new(
            ConfigWriteErrorCode::ConfigValidationError,
            err.to_string(),
        )),
    }
}

pub(crate) fn apply_protocol_edit(
    doc: &mut DocumentMut,
    segments: &[String],
    value: JsonValue,
    merge_strategy: MergeStrategy,
) -> Result<(), ConfigManagerError> {
    let value = toml_item_from_json(value)?;
    match merge_strategy {
        MergeStrategy::Replace => insert_path(doc, segments, value),
        MergeStrategy::Upsert => upsert_path(doc, segments, value),
    }
    Ok(())
}

pub(crate) fn parse_key_path(key_path: &str) -> Result<Vec<String>, ConfigManagerError> {
    let segments = key_path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return Err(ConfigManagerError::new(
            ConfigWriteErrorCode::ConfigPathNotFound,
            "config key path must not be empty",
        ));
    }
    Ok(segments)
}

fn toml_item_from_json(value: JsonValue) -> Result<TomlItem, ConfigManagerError> {
    let mut wrapper = toml::map::Map::new();
    wrapper.insert("value".to_string(), json_to_toml(value));
    let raw = toml::to_string(&TomlValue::Table(wrapper)).map_err(|err| {
        ConfigManagerError::new(ConfigWriteErrorCode::ConfigValidationError, err.to_string())
    })?;
    let mut doc = raw.parse::<DocumentMut>().map_err(|err| {
        ConfigManagerError::new(
            ConfigWriteErrorCode::ConfigValidationError,
            format!("failed to convert JSON value to TOML: {err}"),
        )
    })?;
    doc.as_table_mut().remove("value").ok_or_else(|| {
        ConfigManagerError::new(
            ConfigWriteErrorCode::ConfigValidationError,
            "failed to convert JSON value to TOML",
        )
    })
}

fn insert_path(doc: &mut DocumentMut, segments: &[String], value: TomlItem) {
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let parent = descend(doc, parents);
    parent[last] = value;
}

fn upsert_path(doc: &mut DocumentMut, segments: &[String], value: TomlItem) {
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let parent = descend(doc, parents);
    if parent.contains_key(last.as_str()) {
        merge_item(&mut parent[last], value);
    } else {
        parent[last] = value;
    }
}

fn merge_item(target: &mut TomlItem, incoming: TomlItem) {
    match (target.as_table_mut(), incoming.as_table()) {
        (Some(target_table), Some(incoming_table)) => {
            for (key, value) in incoming_table {
                if target_table.contains_key(key) {
                    merge_item(&mut target_table[key], value.clone());
                } else {
                    target_table[key] = value.clone();
                }
            }
        }
        _ => {
            *target = incoming;
        }
    }
}

fn descend<'a>(doc: &'a mut DocumentMut, segments: &[String]) -> &'a mut TomlTable {
    let mut current = doc.as_table_mut();
    for segment in segments {
        if !current.contains_key(segment.as_str()) {
            current.insert(segment, TomlItem::Table(new_implicit_table()));
        }
        let item = current.get_mut(segment.as_str()).expect("inserted table");
        current = ensure_table_for_write(item);
    }
    current
}

fn ensure_table_for_write(item: &mut TomlItem) -> &mut TomlTable {
    match item {
        TomlItem::Table(_) => {}
        TomlItem::Value(value) => {
            let table = value
                .as_inline_table()
                .map_or_else(new_implicit_table, table_from_inline);
            *item = TomlItem::Table(table);
        }
        TomlItem::None => {
            *item = TomlItem::Table(new_implicit_table());
        }
        _ => {
            *item = TomlItem::Table(new_implicit_table());
        }
    }
    item.as_table_mut().expect("item is table")
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
