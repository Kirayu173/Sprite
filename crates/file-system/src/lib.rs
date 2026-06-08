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
        path: &AbsolutePathBuf,
        _sandbox: Option<&FileSystemSandboxContext>,
    ) -> FileSystemResult<Vec<ReadDirectoryEntry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path.as_path())? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            entries.push(ReadDirectoryEntry {
                path: AbsolutePathBuf::from_absolute_path(entry.path())?,
                metadata: FileMetadata {
                    is_directory: metadata.is_dir(),
                },
            });
        }
        Ok(entries)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct LocalFileSystem;

    impl ExecutorFileSystem for LocalFileSystem {}

    #[tokio::test]
    async fn reads_writes_and_lists_real_files() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = AbsolutePathBuf::from_absolute_path(tempdir.path()).expect("absolute tempdir");
        let fs = LocalFileSystem;
        let file = fs
            .join(&root, Path::new("config.toml"))
            .await
            .expect("join");

        fs.write_file(&file, b"model = \"local\"".to_vec(), None)
            .await
            .expect("write");
        let contents = fs.read_file_text(&file, None).await.expect("read text");
        let entries = fs.read_directory(&root, None).await.expect("read dir");

        assert_eq!(contents, "model = \"local\"");
        assert!(entries.iter().any(|entry| entry.path == file));
    }
}
