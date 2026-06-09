use app_protocol::ConfigLayerMetadata;
use app_protocol::OverriddenMetadata;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

use crate::ConfigLayerStack;

pub(crate) fn first_overridden_metadata(
    stack: &ConfigLayerStack,
    target_metadata: &ConfigLayerMetadata,
    edited_paths: &[Vec<String>],
) -> Option<OverriddenMetadata> {
    let origins = stack.origins();
    let effective_config = stack.effective_config();
    for edited_path in edited_paths {
        for path in origin_paths_for_edit(&effective_config, edited_path) {
            let origin = origins.get(&path)?;
            if origin != target_metadata {
                return Some(OverriddenMetadata {
                    message: format!(
                        "`{}` was written but is overridden by a higher-precedence config layer",
                        edited_path.join(".")
                    ),
                    overriding_layer: origin.clone(),
                    effective_value: json_value_at_path(&effective_config, edited_path)
                        .unwrap_or(JsonValue::Null),
                });
            }
        }
    }
    None
}

fn origin_paths_for_edit(config: &TomlValue, edited_path: &[String]) -> Vec<String> {
    let Some(value) = toml_value_at_path(config, edited_path) else {
        return vec![edited_path.join(".")];
    };
    let mut paths = Vec::new();
    collect_leaf_paths(value, edited_path.to_vec(), &mut paths);
    if paths.is_empty() {
        paths.push(edited_path.join("."));
    }
    paths
}

fn collect_leaf_paths(value: &TomlValue, path: Vec<String>, paths: &mut Vec<String>) {
    match value {
        TomlValue::Table(table) => {
            for (key, value) in table {
                let mut child_path = path.clone();
                child_path.push(key.clone());
                collect_leaf_paths(value, child_path, paths);
            }
        }
        TomlValue::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                let mut child_path = path.clone();
                child_path.push(index.to_string());
                collect_leaf_paths(value, child_path, paths);
            }
        }
        _ => paths.push(path.join(".")),
    }
}

fn toml_value_at_path<'a>(config: &'a TomlValue, path: &[String]) -> Option<&'a TomlValue> {
    let mut current = config;
    for segment in path {
        current = current.as_table()?.get(segment)?;
    }
    Some(current)
}

fn json_value_at_path(config: &TomlValue, path: &[String]) -> Option<JsonValue> {
    let value = toml_value_at_path(config, path)?;
    serde_json::to_value(value).ok()
}
