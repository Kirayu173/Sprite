use utils_absolute_path::AbsolutePathBuf;

pub async fn resolve_root_git_project_for_trust<T>(
    _fs: &T,
    cwd: &AbsolutePathBuf,
) -> Option<AbsolutePathBuf>
where
    T: ?Sized,
{
    let base = if cwd.as_path().is_dir() {
        cwd.clone()
    } else {
        cwd.parent()?
    };
    for ancestor in base.ancestors() {
        if ancestor.join(".git").as_path().exists() {
            return Some(ancestor);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_nearest_ancestor_git_root_for_trust() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        std::fs::create_dir(root.join(".git")).expect(".git");
        std::fs::create_dir_all(root.join("nested").join("project")).expect("nested");
        let cwd = AbsolutePathBuf::from_absolute_path(root.join("nested").join("project"))
            .expect("absolute cwd");

        let resolved = resolve_root_git_project_for_trust(&(), &cwd)
            .await
            .expect("git root");

        assert_eq!(
            resolved,
            AbsolutePathBuf::from_absolute_path(root).expect("absolute root")
        );
    }
}
