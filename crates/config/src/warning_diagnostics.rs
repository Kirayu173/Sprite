use crate::CONFIG_TOML_FILE;
use utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWarning {
    pub message: String,
}

pub fn project_ignored_config_keys_warning(
    dot_sprite_folder: &AbsolutePathBuf,
    ignored_keys: &[String],
) -> ConfigWarning {
    let config_path = dot_sprite_folder.join(CONFIG_TOML_FILE);
    let ignored_keys = ignored_keys.join(", ");
    ConfigWarning {
        message: format!(
            concat!(
                "Ignored unsupported project-local config keys in {config_path}: {ignored_keys}. ",
                "If you want these settings to apply, manually set them in your ",
                "user-level config.toml."
            ),
            config_path = config_path.display(),
            ignored_keys = ignored_keys,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use utils_absolute_path::AbsolutePathBuf;

    #[test]
    fn project_ignored_keys_warning_includes_config_location_and_keys() {
        let folder = if cfg!(windows) {
            AbsolutePathBuf::try_from(std::path::PathBuf::from(r"C:\work\.sprite"))
                .expect("absolute path")
        } else {
            AbsolutePathBuf::try_from(std::path::PathBuf::from("/work/.sprite"))
                .expect("absolute path")
        };

        let warning =
            project_ignored_config_keys_warning(&folder, &["profile".into(), "notify".into()]);

        assert!(warning.message.contains("config.toml"));
        assert!(warning.message.contains("profile, notify"));
        assert!(warning.message.contains("user-level config.toml"));
    }
}
