use dirs::home_dir;
use std::path::PathBuf;
use utils_absolute_path::AbsolutePathBuf;

/// Returns the path to the Sprite configuration directory, which can be
/// specified by the `SPRITE_HOME` environment variable. If not set, defaults to
/// `~/.sprite`.
///
/// - If `SPRITE_HOME` is set, the value must exist and be a directory. The
///   value will be canonicalized and this function will Err otherwise.
/// - If `SPRITE_HOME` is not set, this function does not verify that the
///   directory exists.
pub fn find_sprite_home() -> std::io::Result<AbsolutePathBuf> {
    let sprite_home_env = std::env::var("SPRITE_HOME")
        .ok()
        .filter(|val| !val.is_empty());
    find_sprite_home_from_env(sprite_home_env.as_deref())
}

fn find_sprite_home_from_env(sprite_home_env: Option<&str>) -> std::io::Result<AbsolutePathBuf> {
    // Honor the `SPRITE_HOME` environment variable when it is set to allow users
    // (and tests) to override the default location.
    match sprite_home_env {
        Some(val) => {
            let path = PathBuf::from(val);
            let metadata = std::fs::metadata(&path).map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("SPRITE_HOME points to {val:?}, but that path does not exist"),
                ),
                _ => std::io::Error::new(
                    err.kind(),
                    format!("failed to read SPRITE_HOME {val:?}: {err}"),
                ),
            })?;

            if !metadata.is_dir() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("SPRITE_HOME points to {val:?}, but that path is not a directory"),
                ))
            } else {
                let canonical = path.canonicalize().map_err(|err| {
                    std::io::Error::new(
                        err.kind(),
                        format!("failed to canonicalize SPRITE_HOME {val:?}: {err}"),
                    )
                })?;
                AbsolutePathBuf::from_absolute_path(canonical)
            }
        }
        None => {
            let mut p = home_dir().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find home directory",
                )
            })?;
            p.push(".sprite");
            AbsolutePathBuf::from_absolute_path(p)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::find_sprite_home_from_env;
    use dirs::home_dir;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::io::ErrorKind;
    use tempfile::TempDir;
    use utils_absolute_path::AbsolutePathBuf;

    #[test]
    fn find_sprite_home_env_missing_path_is_fatal() {
        let temp_home = TempDir::new().expect("temp home");
        let missing = temp_home.path().join("missing-sprite-home");
        let missing_str = missing
            .to_str()
            .expect("missing sprite home path should be valid utf-8");

        let err = find_sprite_home_from_env(Some(missing_str)).expect_err("missing SPRITE_HOME");
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(
            err.to_string().contains("SPRITE_HOME"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_sprite_home_env_file_path_is_fatal() {
        let temp_home = TempDir::new().expect("temp home");
        let file_path = temp_home.path().join("sprite-home.txt");
        fs::write(&file_path, "not a directory").expect("write temp file");
        let file_str = file_path
            .to_str()
            .expect("file sprite home path should be valid utf-8");

        let err = find_sprite_home_from_env(Some(file_str)).expect_err("file SPRITE_HOME");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("not a directory"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_sprite_home_env_valid_directory_canonicalizes() {
        let temp_home = TempDir::new().expect("temp home");
        let temp_str = temp_home
            .path()
            .to_str()
            .expect("temp sprite home path should be valid utf-8");

        let resolved = find_sprite_home_from_env(Some(temp_str)).expect("valid SPRITE_HOME");
        let expected = temp_home
            .path()
            .canonicalize()
            .expect("canonicalize temp home");
        let expected = AbsolutePathBuf::from_absolute_path(expected).expect("absolute home");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn find_sprite_home_without_env_uses_default_home_dir() {
        let resolved =
            find_sprite_home_from_env(/*sprite_home_env*/ None).expect("default SPRITE_HOME");
        let mut expected = home_dir().expect("home dir");
        expected.push(".sprite");
        let expected = AbsolutePathBuf::from_absolute_path(expected).expect("absolute home");
        assert_eq!(resolved, expected);
    }
}
