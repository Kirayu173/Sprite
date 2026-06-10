use async_trait::async_trait;
use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::PermissionProfile;
use runtime_protocol::models::SandboxEnforcement;
use runtime_protocol::permissions::FileSystemPath;
use runtime_protocol::permissions::FileSystemSandboxKind;
use runtime_protocol::permissions::FileSystemSandboxPolicy;
use runtime_protocol::permissions::FileSystemSpecialPath;
use runtime_protocol::permissions::NetworkSandboxPolicy;
use runtime_protocol::protocol::SandboxPolicy;
use std::io;
use std::path::Path;
use utils_absolute_path::AbsolutePathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateDirectoryOptions {
    pub recursive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoveOptions {
    pub recursive: bool,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyOptions {
    pub recursive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    pub is_directory: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub created_at_ms: i64,
    pub modified_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadDirectoryEntry {
    pub file_name: String,
    pub is_directory: bool,
    pub is_file: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemSandboxContext {
    pub permissions: PermissionProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<AbsolutePathBuf>,
    pub windows_sandbox_level: WindowsSandboxLevel,
    #[serde(default)]
    pub windows_sandbox_private_desktop: bool,
    #[serde(default)]
    pub use_legacy_landlock: bool,
}

impl FileSystemSandboxContext {
    pub fn from_legacy_sandbox_policy(sandbox_policy: SandboxPolicy, cwd: AbsolutePathBuf) -> Self {
        let file_system_sandbox_policy =
            FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(&sandbox_policy, &cwd);
        let permissions = PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::from_legacy_sandbox_policy(&sandbox_policy),
            &file_system_sandbox_policy,
            NetworkSandboxPolicy::from(&sandbox_policy),
        );
        Self::from_permission_profile_with_cwd(permissions, cwd)
    }

    pub fn from_permission_profile(permissions: PermissionProfile) -> Self {
        Self::from_permissions_and_cwd(permissions, /*cwd*/ None)
    }

    pub fn from_permission_profile_with_cwd(
        permissions: PermissionProfile,
        cwd: AbsolutePathBuf,
    ) -> Self {
        Self::from_permissions_and_cwd(permissions, Some(cwd))
    }

    fn from_permissions_and_cwd(
        permissions: PermissionProfile,
        cwd: Option<AbsolutePathBuf>,
    ) -> Self {
        Self {
            permissions,
            cwd,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
            windows_sandbox_private_desktop: false,
            use_legacy_landlock: false,
        }
    }

    pub fn should_run_in_sandbox(&self) -> bool {
        let file_system_policy = self.permissions.file_system_sandbox_policy();
        matches!(file_system_policy.kind, FileSystemSandboxKind::Restricted)
            && !file_system_policy.has_full_disk_write_access()
    }

    pub fn has_cwd_dependent_permissions(&self) -> bool {
        let file_system_policy = self.permissions.file_system_sandbox_policy();
        file_system_policy_has_cwd_dependent_entries(&file_system_policy)
    }

    pub fn drop_cwd_if_unused(mut self) -> Self {
        if !self.has_cwd_dependent_permissions() {
            self.cwd = None;
        }
        self
    }
}

fn file_system_policy_has_cwd_dependent_entries(
    file_system_policy: &FileSystemSandboxPolicy,
) -> bool {
    file_system_policy
        .entries
        .iter()
        .any(|entry| match &entry.path {
            FileSystemPath::GlobPattern { pattern } => !Path::new(pattern).is_absolute(),
            FileSystemPath::Special {
                value: FileSystemSpecialPath::ProjectRoots { .. },
            } => true,
            FileSystemPath::Path { .. } | FileSystemPath::Special { .. } => false,
        })
}

pub type FileSystemResult<T> = io::Result<T>;

/// Abstract filesystem access used by components that may operate locally or via
/// a remote environment.
#[async_trait]
pub trait ExecutorFileSystem: Send + Sync {
    /// Resolves a path within this filesystem.
    async fn canonicalize(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<AbsolutePathBuf> {
        AbsolutePathBuf::from_absolute_path(path.as_path().canonicalize()?)
    }

    /// Lexically joins a path onto an existing bound path.
    async fn join(
        &self,
        base_path: &AbsolutePathBuf,
        path: &Path,
    ) -> FileSystemResult<AbsolutePathBuf> {
        Ok(base_path.join(path))
    }

    /// Returns the parent directory of a bound path.
    async fn parent(&self, path: &AbsolutePathBuf) -> FileSystemResult<Option<AbsolutePathBuf>> {
        Ok(path.parent())
    }

    async fn read_file(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<Vec<u8>> {
        std::fs::read(path.as_path())
    }

    /// Reads a file and decodes it as UTF-8 text.
    async fn read_file_text(
        &self,
        path: &AbsolutePathBuf,
        sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<String> {
        let bytes = self.read_file(path, sandbox).await?;
        String::from_utf8(bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }

    async fn write_file(
        &self,
        path: &AbsolutePathBuf,
        contents: Vec<u8>,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        std::fs::write(path.as_path(), contents)
    }

    async fn create_directory(
        &self,
        path: &AbsolutePathBuf,
        create_directory_options: CreateDirectoryOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        if create_directory_options.recursive {
            std::fs::create_dir_all(path.as_path())
        } else {
            std::fs::create_dir(path.as_path())
        }
    }

    async fn get_metadata(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<FileMetadata> {
        let metadata = std::fs::symlink_metadata(path.as_path())?;
        let file_type = metadata.file_type();
        let created_at_ms = metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        let modified_at_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        Ok(FileMetadata {
            is_directory: metadata.is_dir(),
            is_file: metadata.is_file(),
            is_symlink: file_type.is_symlink(),
            created_at_ms,
            modified_at_ms,
        })
    }

    async fn read_directory(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<Vec<ReadDirectoryEntry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path.as_path())? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            entries.push(ReadDirectoryEntry {
                file_name: entry.file_name().to_string_lossy().to_string(),
                is_directory: metadata.is_dir(),
                is_file: metadata.is_file(),
            });
        }
        Ok(entries)
    }

    async fn remove(
        &self,
        path: &AbsolutePathBuf,
        remove_options: RemoveOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        let metadata = std::fs::symlink_metadata(path.as_path())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            if remove_options.recursive {
                std::fs::remove_dir_all(path.as_path())
            } else {
                std::fs::remove_dir(path.as_path())
            }
        } else if remove_options.force {
            match std::fs::remove_file(path.as_path()) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            }
        } else {
            std::fs::remove_file(path.as_path())
        }
    }

    async fn copy(
        &self,
        source_path: &AbsolutePathBuf,
        destination_path: &AbsolutePathBuf,
        copy_options: CopyOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        if copy_options.recursive {
            copy_recursively(source_path.as_path(), destination_path.as_path())
        } else {
            std::fs::copy(source_path.as_path(), destination_path.as_path()).map(|_| ())
        }
    }
}

fn copy_recursively(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, destination).map(|_| ())
    }
}
