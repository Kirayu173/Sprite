use async_trait::async_trait;
use std::io;
use std::path::Path;
use utils_absolute_path::AbsolutePathBuf;

pub type FileSystemResult<T> = io::Result<T>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSystemSandboxContext;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopyOptions;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateDirectoryOptions;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveOptions;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileMetadata {
    pub is_directory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadDirectoryEntry {
    pub path: AbsolutePathBuf,
    pub metadata: FileMetadata,
}

#[async_trait]
pub trait ExecutorFileSystem: Send + Sync {
    async fn canonicalize(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<AbsolutePathBuf> {
        AbsolutePathBuf::from_absolute_path(path.as_path().canonicalize()?)
    }

    async fn resolve_path(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<AbsolutePathBuf> {
        Ok(path.clone())
    }

    async fn join(
        &self,
        base_path: &AbsolutePathBuf,
        path: &Path,
    ) -> FileSystemResult<AbsolutePathBuf> {
        Ok(base_path.join(path))
    }

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
        _options: CreateDirectoryOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        std::fs::create_dir_all(path.as_path())
    }

    async fn get_metadata(
        &self,
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<FileMetadata> {
        let metadata = std::fs::metadata(path.as_path())?;
        Ok(FileMetadata {
            is_directory: metadata.is_dir(),
        })
    }

    async fn read_directory(
        &self,
        _path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<Vec<ReadDirectoryEntry>> {
        Ok(Vec::new())
    }

    async fn remove(
        &self,
        path: &AbsolutePathBuf,
        _options: RemoveOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        std::fs::remove_file(path.as_path())
    }

    async fn copy(
        &self,
        from: &AbsolutePathBuf,
        to: &AbsolutePathBuf,
        _options: CopyOptions,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<()> {
        std::fs::copy(from.as_path(), to.as_path()).map(|_| ())
    }
}
