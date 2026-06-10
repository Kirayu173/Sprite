mod info;
pub use info::GitInfo;
pub use info::collect_git_info;
pub use info::get_git_repo_root;
pub use info::get_git_repo_root_with_fs;
pub use info::resolve_root_git_project_for_trust;
pub use runtime_protocol::protocol::GitSha;
