use utils_absolute_path::AbsolutePathBuf;

pub async fn resolve_root_git_project_for_trust<T>(
    _fs: &T,
    _cwd: &AbsolutePathBuf,
) -> Option<AbsolutePathBuf>
where
    T: ?Sized,
{
    None
}
