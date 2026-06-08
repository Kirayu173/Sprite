use app_protocol::ConfigLayerSource;

pub fn format_config_layer_source(source: &ConfigLayerSource, config_toml_file: &str) -> String {
    match source {
        ConfigLayerSource::System { file } => {
            format!("system ({})", file.as_path().display())
        }
        ConfigLayerSource::User { file, .. } => {
            format!("user ({})", file.as_path().display())
        }
        ConfigLayerSource::Project { project_config_dir } => {
            format!(
                "project ({}/{config_toml_file})",
                project_config_dir.as_path().display()
            )
        }
        ConfigLayerSource::SessionFlags => "session-flags".to_string(),
    }
}
