use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use runtime_protocol::models::PermissionProfile;
use utils_absolute_path::AbsolutePathBuf;

use crate::windows_sandbox::run_setup_refresh_with_extra_read_roots;

pub fn grant_read_root_non_elevated(
    permission_profile: &PermissionProfile,
    workspace_roots: &[AbsolutePathBuf],
    command_cwd: &Path,
    env_map: &HashMap<String, String>,
    sprite_home: &Path,
    read_root: &Path,
) -> Result<PathBuf> {
    if !read_root.is_absolute() {
        anyhow::bail!("path must be absolute: {}", read_root.display());
    }
    if !read_root.exists() {
        anyhow::bail!("path does not exist: {}", read_root.display());
    }
    if !read_root.is_dir() {
        anyhow::bail!("path must be a directory: {}", read_root.display());
    }

    let canonical_root = dunce::canonicalize(read_root)?;
    run_setup_refresh_with_extra_read_roots(
        permission_profile,
        workspace_roots,
        command_cwd,
        env_map,
        sprite_home,
        vec![canonical_root.clone()],
    )?;
    Ok(canonical_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_path() {
        let profile = PermissionProfile::read_only();
        let err = grant_read_root_non_elevated(
            &profile,
            &[],
            std::env::current_dir().unwrap().as_path(),
            &HashMap::new(),
            std::env::temp_dir().as_path(),
            Path::new("relative"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("absolute"));
    }
}
