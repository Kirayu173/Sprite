use std::ffi::OsStr;

use file_system::ExecutorFileSystem;
use utils_absolute_path::AbsolutePathBuf;

/// Resolve the path that should be used for project trust checks.
///
/// Regular checkouts resolve to their worktree root. Linked worktrees resolve
/// to the main checkout root by following the `.git` file's `gitdir:` pointer
/// back through `.git/worktrees/<name>`.
pub async fn resolve_root_git_project_for_trust(
    fs: &dyn ExecutorFileSystem,
    cwd: &AbsolutePathBuf,
) -> Option<AbsolutePathBuf> {
    let (repo_root, dot_git) = find_ancestor_git_entry_with_fs(fs, cwd).await?;
    if fs
        .get_metadata(&dot_git, /*sandbox*/ None)
        .await
        .ok()?
        .is_directory
    {
        return Some(repo_root);
    }

    let git_dir = parse_gitdir_file(fs, &dot_git, &repo_root).await?;
    let worktrees_dir = git_dir.parent()?;
    if worktrees_dir.as_path().file_name() != Some(OsStr::new("worktrees")) {
        return None;
    }

    let common_dir = worktrees_dir.parent()?;
    common_dir.parent()
}

async fn find_ancestor_git_entry_with_fs(
    fs: &dyn ExecutorFileSystem,
    cwd: &AbsolutePathBuf,
) -> Option<(AbsolutePathBuf, AbsolutePathBuf)> {
    let base = match fs.get_metadata(cwd, /*sandbox*/ None).await {
        Ok(metadata) if metadata.is_directory => cwd.clone(),
        _ => cwd.parent()?,
    };

    for dir in base.ancestors() {
        let dot_git = dir.join(".git");
        if fs.get_metadata(&dot_git, /*sandbox*/ None).await.is_ok() {
            return Some((dir, dot_git));
        }
    }
    None
}

async fn parse_gitdir_file(
    fs: &dyn ExecutorFileSystem,
    dot_git: &AbsolutePathBuf,
    repo_root: &AbsolutePathBuf,
) -> Option<AbsolutePathBuf> {
    let contents = fs.read_file_text(dot_git, /*sandbox*/ None).await.ok()?;
    let git_dir = contents.trim().strip_prefix("gitdir:")?.trim();
    if git_dir.is_empty() {
        return None;
    }
    Some(AbsolutePathBuf::resolve_path_against_base(
        git_dir,
        repo_root.as_path(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct LocalFileSystem;

    impl ExecutorFileSystem for LocalFileSystem {}

    fn abs(path: impl AsRef<std::path::Path>) -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path(path.as_ref()).expect("absolute path")
    }

    #[tokio::test]
    async fn resolves_nearest_ancestor_git_root_for_trust() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        std::fs::create_dir(root.join(".git")).expect(".git");
        std::fs::create_dir_all(root.join("nested").join("project")).expect("nested");
        let cwd = abs(root.join("nested").join("project"));

        let resolved = resolve_root_git_project_for_trust(&LocalFileSystem, &cwd)
            .await
            .expect("git root");

        assert_eq!(resolved, abs(root));
    }

    #[tokio::test]
    async fn resolves_nested_file_to_repo_root() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        std::fs::create_dir(root.join(".git")).expect(".git");
        std::fs::create_dir_all(root.join("nested")).expect("nested");
        let file = root.join("nested").join("README.md");
        std::fs::write(&file, "readme").expect("file");

        let resolved = resolve_root_git_project_for_trust(&LocalFileSystem, &abs(&file))
            .await
            .expect("git root");

        assert_eq!(resolved, abs(root));
    }

    #[tokio::test]
    async fn linked_worktree_returns_main_checkout_root() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo_root = tempdir.path().join("repo");
        let worktree_root = tempdir.path().join("worktree");
        let worktree_git_dir = repo_root.join(".git").join("worktrees").join("feature-x");
        std::fs::create_dir_all(&worktree_git_dir).expect("worktree git dir");
        std::fs::create_dir_all(worktree_root.join("nested")).expect("worktree nested");
        std::fs::write(
            worktree_root.join(".git"),
            format!("gitdir: {}\n", worktree_git_dir.display()),
        )
        .expect(".git file");

        let resolved = resolve_root_git_project_for_trust(
            &LocalFileSystem,
            &abs(worktree_root.join("nested")),
        )
        .await
        .expect("main checkout root");

        assert_eq!(resolved, abs(repo_root));
    }

    #[tokio::test]
    async fn non_worktree_gitdir_file_returns_none() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let project = tempdir.path().join("project");
        std::fs::create_dir_all(&project).expect("project");
        std::fs::write(project.join(".git"), "gitdir: /tmp/fake-git-dir\n").expect(".git file");

        let resolved = resolve_root_git_project_for_trust(&LocalFileSystem, &abs(&project)).await;

        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn invalid_gitdir_file_returns_none() -> io::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let project = tempdir.path().join("project");
        std::fs::create_dir_all(&project)?;
        std::fs::write(project.join(".git"), "not a gitdir pointer\n")?;

        let resolved = resolve_root_git_project_for_trust(&LocalFileSystem, &abs(&project)).await;

        assert_eq!(resolved, None);
        Ok(())
    }
}
