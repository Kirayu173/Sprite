use std::path::Path;
use std::sync::Arc;

use tokio::sync::watch;
use utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSnapshot {
    pub path: AbsolutePathBuf,
    pub cwd: AbsolutePathBuf,
}

impl ShellSnapshot {
    pub fn capture(
        shell_path: impl AsRef<Path>,
        cwd: impl AsRef<Path>,
    ) -> Result<Self, ShellSnapshotError> {
        let path = AbsolutePathBuf::from_absolute_path(shell_path.as_ref().to_path_buf()).map_err(
            |_| ShellSnapshotError::RelativeShellPath(shell_path.as_ref().to_path_buf()),
        )?;
        let cwd = AbsolutePathBuf::from_absolute_path(cwd.as_ref().to_path_buf())
            .map_err(|_| ShellSnapshotError::RelativeCwd(cwd.as_ref().to_path_buf()))?;
        let snapshot = Self { path, cwd };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), ShellSnapshotError> {
        if !self.path.as_path().is_file() {
            return Err(ShellSnapshotError::MissingShellPath(
                self.path.as_path().to_path_buf(),
            ));
        }
        if !self.cwd.as_path().is_dir() {
            return Err(ShellSnapshotError::MissingCwd(
                self.cwd.as_path().to_path_buf(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShellSnapshotError {
    #[error("shell path must be absolute: {0}")]
    RelativeShellPath(std::path::PathBuf),
    #[error("cwd must be absolute: {0}")]
    RelativeCwd(std::path::PathBuf),
    #[error("shell path does not exist or is not a file: {0}")]
    MissingShellPath(std::path::PathBuf),
    #[error("cwd does not exist or is not a directory: {0}")]
    MissingCwd(std::path::PathBuf),
}

#[derive(Debug, Clone)]
pub struct ShellSnapshotHandle {
    tx: watch::Sender<Option<Arc<ShellSnapshot>>>,
}

impl Default for ShellSnapshotHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellSnapshotHandle {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(None);
        Self { tx }
    }

    pub fn receiver(&self) -> watch::Receiver<Option<Arc<ShellSnapshot>>> {
        self.tx.subscribe()
    }

    pub fn current(&self) -> Option<Arc<ShellSnapshot>> {
        self.tx.borrow().clone()
    }

    pub fn update(&self, snapshot: ShellSnapshot) {
        let _ = self.tx.send(Some(Arc::new(snapshot)));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(None);
    }
}

pub fn empty_shell_snapshot_receiver() -> watch::Receiver<Option<Arc<ShellSnapshot>>> {
    ShellSnapshotHandle::new().receiver()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_valid_absolute_snapshot() {
        let shell = std::env::current_exe().expect("current exe");
        let cwd = std::env::current_dir().expect("current dir");

        let snapshot = ShellSnapshot::capture(shell.clone(), cwd.clone()).expect("capture");

        assert_eq!(snapshot.path.as_path(), shell.as_path());
        assert_eq!(snapshot.cwd.as_path(), cwd.as_path());
    }

    #[test]
    fn rejects_missing_shell_path() {
        let missing = std::env::current_dir()
            .expect("current dir")
            .join("missing-shell-for-test");

        let err = ShellSnapshot::capture(missing, std::env::current_dir().expect("current dir"))
            .expect_err("missing shell should fail");
        assert!(matches!(err, ShellSnapshotError::MissingShellPath(_)));
    }

    #[test]
    fn handle_updates_and_clears_snapshot() {
        let handle = ShellSnapshotHandle::new();
        let rx = handle.receiver();
        assert!(rx.borrow().is_none());

        handle.update(
            ShellSnapshot::capture(
                std::env::current_exe().expect("current exe"),
                std::env::current_dir().expect("current dir"),
            )
            .expect("capture"),
        );
        assert!(rx.borrow().is_some());

        handle.clear();
        assert!(rx.borrow().is_none());
    }
}
