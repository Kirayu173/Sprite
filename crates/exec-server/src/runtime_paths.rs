use std::path::PathBuf;

use utils_absolute_path::AbsolutePathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecServerRuntimePaths {
    pub sprite_self_exe: AbsolutePathBuf,
    pub sprite_linux_sandbox_exe: Option<AbsolutePathBuf>,
}

impl ExecServerRuntimePaths {
    pub fn from_optional_paths(
        sprite_self_exe: Option<PathBuf>,
        sprite_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let sprite_self_exe = sprite_self_exe.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Sprite executable path is not configured",
            )
        })?;
        Self::new(sprite_self_exe, sprite_linux_sandbox_exe)
    }

    pub fn new(
        sprite_self_exe: PathBuf,
        sprite_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            sprite_self_exe: absolute_path(sprite_self_exe)?,
            sprite_linux_sandbox_exe: sprite_linux_sandbox_exe.map(absolute_path).transpose()?,
        })
    }
}

fn absolute_path(path: PathBuf) -> std::io::Result<AbsolutePathBuf> {
    AbsolutePathBuf::from_absolute_path(path.as_path())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
}
