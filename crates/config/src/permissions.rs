use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::permissions_toml::FilesystemPermissionToml;
use crate::permissions_toml::FilesystemPermissionsToml;
use crate::permissions_toml::NetworkToml;
use crate::permissions_toml::PermissionProfileToml;
use crate::permissions_toml::PermissionsToml;
use crate::permissions_toml::WorkspaceRootsToml;
use crate::types::SandboxWorkspaceWrite;
use runtime_protocol::config_types::WindowsSandboxLevel;
use runtime_protocol::models::ActivePermissionProfile;
use runtime_protocol::models::BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS;
use runtime_protocol::models::BUILT_IN_PERMISSION_PROFILE_READ_ONLY;
use runtime_protocol::models::BUILT_IN_PERMISSION_PROFILE_WORKSPACE;
use runtime_protocol::models::PermissionProfile;
use runtime_protocol::permissions::FileSystemAccessMode;
use runtime_protocol::permissions::FileSystemPath;
use runtime_protocol::permissions::FileSystemSandboxEntry;
use runtime_protocol::permissions::FileSystemSandboxPolicy;
use runtime_protocol::permissions::FileSystemSpecialPath;
use runtime_protocol::permissions::NetworkSandboxPolicy;
use runtime_protocol::permissions::project_roots_glob_pattern;
use utils_absolute_path::AbsolutePathBuf;

use crate::config_toml::ProjectConfig;

pub const BUILT_IN_READ_ONLY_PROFILE: &str = BUILT_IN_PERMISSION_PROFILE_READ_ONLY;
pub const BUILT_IN_WORKSPACE_PROFILE: &str = BUILT_IN_PERMISSION_PROFILE_WORKSPACE;
pub const BUILT_IN_DANGER_FULL_ACCESS_PROFILE: &str =
    BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPermissionProfile {
    pub profile: PermissionProfile,
    pub active_profile: ActivePermissionProfile,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub warnings: Vec<String>,
}

pub fn default_builtin_permission_profile_name(
    active_project: Option<&ProjectConfig>,
    windows_sandbox_level: WindowsSandboxLevel,
) -> &'static str {
    if active_project.is_some_and(|project| project.is_trusted() || project.is_untrusted())
        && !(cfg!(target_os = "windows") && windows_sandbox_level == WindowsSandboxLevel::Disabled)
    {
        BUILT_IN_WORKSPACE_PROFILE
    } else {
        BUILT_IN_READ_ONLY_PROFILE
    }
}

pub fn is_builtin_permission_profile_name(profile_name: &str) -> bool {
    matches!(
        profile_name,
        BUILT_IN_READ_ONLY_PROFILE
            | BUILT_IN_WORKSPACE_PROFILE
            | BUILT_IN_DANGER_FULL_ACCESS_PROFILE
    )
}

pub fn validate_user_permission_profile_names(
    permissions: Option<&PermissionsToml>,
) -> io::Result<()> {
    let Some(permissions) = permissions else {
        return Ok(());
    };

    for profile_name in permissions.entries.keys() {
        if profile_name.starts_with(':') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "permissions profile `{profile_name}` uses a reserved built-in profile prefix"
                ),
            ));
        }
    }

    Ok(())
}

pub fn resolve_effective_permission_profile(
    permissions: Option<&PermissionsToml>,
    selected_profile_name: Option<&str>,
    workspace_write: Option<&SandboxWorkspaceWrite>,
    active_project: Option<&ProjectConfig>,
    windows_sandbox_level: WindowsSandboxLevel,
    policy_cwd: &Path,
) -> io::Result<ResolvedPermissionProfile> {
    validate_user_permission_profile_names(permissions)?;
    let profile_name = selected_profile_name.unwrap_or_else(|| {
        default_builtin_permission_profile_name(active_project, windows_sandbox_level)
    });
    let mut warnings = Vec::new();
    let (file_system, network) = compile_permission_profile_selection(
        permissions,
        profile_name,
        workspace_write,
        policy_cwd,
        &mut warnings,
    )?;
    let profile = PermissionProfile::from_runtime_permissions(&file_system, network);
    let mut active_profile = ActivePermissionProfile::new(profile_name);
    if !is_builtin_permission_profile_name(profile_name)
        && let Some(permissions) = permissions
    {
        active_profile.extends = permissions
            .entries
            .get(profile_name)
            .and_then(|profile| profile.extends.clone());
    }
    let workspace_roots =
        compile_permission_profile_workspace_roots(permissions, profile_name, policy_cwd)?;

    Ok(ResolvedPermissionProfile {
        profile,
        active_profile,
        workspace_roots,
        warnings,
    })
}

fn builtin_permission_profile(
    profile_name: &str,
    workspace_write: Option<&SandboxWorkspaceWrite>,
) -> Option<PermissionProfile> {
    match profile_name {
        BUILT_IN_READ_ONLY_PROFILE => Some(PermissionProfile::read_only()),
        BUILT_IN_WORKSPACE_PROFILE => Some(match workspace_write {
            Some(SandboxWorkspaceWrite {
                writable_roots: _,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            }) => PermissionProfile::workspace_write_with(
                &[],
                if *network_access {
                    NetworkSandboxPolicy::Enabled
                } else {
                    NetworkSandboxPolicy::Restricted
                },
                *exclude_tmpdir_env_var,
                *exclude_slash_tmp,
            ),
            None => PermissionProfile::workspace_write(),
        }),
        BUILT_IN_DANGER_FULL_ACCESS_PROFILE => Some(PermissionProfile::Disabled),
        _ => None,
    }
}

fn resolve_permission_profile(
    permissions: &PermissionsToml,
    profile_name: &str,
) -> io::Result<PermissionProfileToml> {
    permissions
        .resolve_profile(profile_name, extensible_builtin_parent_profile)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.to_string()))
}

fn extensible_builtin_parent_profile(profile_name: &str) -> Option<PermissionProfileToml> {
    let file_system = match profile_name {
        BUILT_IN_READ_ONLY_PROFILE => FileSystemSandboxPolicy::read_only(),
        BUILT_IN_WORKSPACE_PROFILE => FileSystemSandboxPolicy::workspace_write(
            &[],
            /*exclude_tmpdir_env_var*/ false,
            /*exclude_slash_tmp*/ false,
        ),
        _ => return None,
    };
    Some(permission_profile_toml_from_file_system_policy(file_system))
}

fn permission_profile_toml_from_file_system_policy(
    file_system: FileSystemSandboxPolicy,
) -> PermissionProfileToml {
    let mut filesystem = FilesystemPermissionsToml {
        glob_scan_max_depth: file_system.glob_scan_max_depth,
        entries: BTreeMap::new(),
    };
    for entry in file_system.entries {
        insert_filesystem_permission_toml(&mut filesystem.entries, entry);
    }
    PermissionProfileToml {
        description: None,
        extends: None,
        workspace_roots: None,
        filesystem: Some(filesystem),
        network: None,
    }
}

fn insert_filesystem_permission_toml(
    entries: &mut BTreeMap<String, FilesystemPermissionToml>,
    entry: FileSystemSandboxEntry,
) {
    match entry.path {
        FileSystemPath::Path { path } => {
            entries.insert(
                path.into_path_buf().to_string_lossy().into_owned(),
                FilesystemPermissionToml::Access(entry.access),
            );
        }
        FileSystemPath::GlobPattern { pattern } => {
            entries.insert(pattern, FilesystemPermissionToml::Access(entry.access));
        }
        FileSystemPath::Special { value } => {
            insert_special_filesystem_permission_toml(entries, value, entry.access);
        }
    }
}

fn insert_special_filesystem_permission_toml(
    entries: &mut BTreeMap<String, FilesystemPermissionToml>,
    value: FileSystemSpecialPath,
    access: FileSystemAccessMode,
) {
    match value {
        FileSystemSpecialPath::Root => {
            entries.insert(
                ":root".to_string(),
                FilesystemPermissionToml::Access(access),
            );
        }
        FileSystemSpecialPath::Minimal => {
            entries.insert(
                ":minimal".to_string(),
                FilesystemPermissionToml::Access(access),
            );
        }
        FileSystemSpecialPath::ProjectRoots { subpath } => {
            insert_scoped_filesystem_permission_toml(
                entries,
                ":workspace_roots".to_string(),
                subpath.unwrap_or_else(|| PathBuf::from(".")),
                access,
            );
        }
        FileSystemSpecialPath::Tmpdir => {
            entries.insert(
                ":tmpdir".to_string(),
                FilesystemPermissionToml::Access(access),
            );
        }
        FileSystemSpecialPath::SlashTmp => {
            entries.insert(
                ":slash_tmp".to_string(),
                FilesystemPermissionToml::Access(access),
            );
        }
        FileSystemSpecialPath::Unknown { path, subpath } => {
            if let Some(subpath) = subpath {
                insert_scoped_filesystem_permission_toml(entries, path, subpath, access);
            } else {
                entries.insert(path, FilesystemPermissionToml::Access(access));
            }
        }
    };
}

fn insert_scoped_filesystem_permission_toml(
    entries: &mut BTreeMap<String, FilesystemPermissionToml>,
    path: String,
    subpath: PathBuf,
    access: FileSystemAccessMode,
) {
    let permission = entries
        .entry(path)
        .or_insert_with(|| FilesystemPermissionToml::Scoped(BTreeMap::new()));
    match permission {
        FilesystemPermissionToml::Scoped(scoped_entries) => {
            scoped_entries.insert(subpath.to_string_lossy().into_owned(), access);
        }
        FilesystemPermissionToml::Access(_) => {
            *permission = FilesystemPermissionToml::Scoped(BTreeMap::from([(
                subpath.to_string_lossy().into_owned(),
                access,
            )]));
        }
    }
}

fn compile_permission_profile_selection(
    permissions: Option<&PermissionsToml>,
    profile_name: &str,
    workspace_write: Option<&SandboxWorkspaceWrite>,
    policy_cwd: &Path,
    startup_warnings: &mut Vec<String>,
) -> io::Result<(FileSystemSandboxPolicy, NetworkSandboxPolicy)> {
    if let Some(permission_profile) = builtin_permission_profile(profile_name, workspace_write) {
        return Ok(permission_profile.to_runtime_permissions());
    }
    reject_unknown_builtin_permission_profile(profile_name)?;

    let permissions = permissions.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "default_permissions requires a `[permissions]` table",
        )
    })?;
    compile_permission_profile(permissions, profile_name, policy_cwd, startup_warnings)
}

fn compile_permission_profile(
    permissions: &PermissionsToml,
    profile_name: &str,
    policy_cwd: &Path,
    startup_warnings: &mut Vec<String>,
) -> io::Result<(FileSystemSandboxPolicy, NetworkSandboxPolicy)> {
    let profile = resolve_permission_profile(permissions, profile_name)?;
    let mut file_system_sandbox_policy = FileSystemSandboxPolicy::restricted(Vec::new());
    let base_network_sandbox_policy = NetworkSandboxPolicy::Restricted;
    if let Some(filesystem) = profile.filesystem.as_ref() {
        if filesystem.is_empty() && file_system_sandbox_policy.entries.is_empty() {
            push_warning(
                startup_warnings,
                missing_filesystem_entries_warning(profile_name),
            );
        } else {
            if cfg!(not(target_os = "macos")) {
                for pattern in unsupported_read_write_glob_paths(filesystem) {
                    push_warning(
                        startup_warnings,
                        format!(
                            "Filesystem glob `{pattern}` uses `read` or `write` access, which is not fully supported by this platform's sandboxing. Use an exact path or trailing `/**` subtree rule instead. `deny` globs are supported."
                        ),
                    );
                }
                for pattern in unbounded_unreadable_globstar_paths(filesystem) {
                    push_warning(
                        startup_warnings,
                        format!(
                            "Filesystem deny-read glob `{pattern}` uses `**`. Non-macOS sandboxing does not support unbounded `**` natively; set `glob_scan_max_depth` in this filesystem profile to cap Linux glob expansion and silence this warning, or enumerate explicit depths such as `*.env`, `*/*.env`, and `*/*/*.env`."
                        ),
                    );
                }
            }
            for (path, permission) in &filesystem.entries {
                file_system_sandbox_policy
                    .entries
                    .extend(compile_filesystem_permission(
                        path,
                        permission,
                        policy_cwd,
                        startup_warnings,
                    )?);
            }
        }
    } else if file_system_sandbox_policy.entries.is_empty() {
        push_warning(
            startup_warnings,
            missing_filesystem_entries_warning(profile_name),
        );
    }
    if let Some(glob_scan_max_depth) = validate_glob_scan_max_depth(
        profile
            .filesystem
            .as_ref()
            .and_then(|filesystem| filesystem.glob_scan_max_depth),
    )? {
        file_system_sandbox_policy.glob_scan_max_depth = Some(glob_scan_max_depth);
    }
    let network_sandbox_policy =
        compile_network_sandbox_policy(profile.network.as_ref(), base_network_sandbox_policy);
    Ok((file_system_sandbox_policy, network_sandbox_policy))
}

fn compile_permission_profile_workspace_roots(
    permissions: Option<&PermissionsToml>,
    profile_name: &str,
    policy_cwd: &Path,
) -> io::Result<Vec<AbsolutePathBuf>> {
    if is_builtin_permission_profile_name(profile_name) {
        return Ok(Vec::new());
    }
    reject_unknown_builtin_permission_profile(profile_name)?;

    let permissions = permissions.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "default_permissions requires a `[permissions]` table",
        )
    })?;
    let profile = resolve_permission_profile(permissions, profile_name)?;
    Ok(compile_workspace_roots(
        profile.workspace_roots.as_ref(),
        policy_cwd,
    ))
}

fn compile_workspace_roots(
    workspace_roots: Option<&WorkspaceRootsToml>,
    policy_cwd: &Path,
) -> Vec<AbsolutePathBuf> {
    workspace_roots.map_or_else(Vec::new, |workspace_roots| {
        workspace_roots
            .enabled_roots()
            .map(|path| AbsolutePathBuf::resolve_path_against_base(path, policy_cwd))
            .collect()
    })
}

fn reject_unknown_builtin_permission_profile(profile_name: &str) -> io::Result<()> {
    if profile_name.starts_with(':') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("default_permissions refers to unknown built-in profile `{profile_name}`"),
        ));
    }

    Ok(())
}

fn compile_network_sandbox_policy(
    network: Option<&NetworkToml>,
    base_network_sandbox_policy: NetworkSandboxPolicy,
) -> NetworkSandboxPolicy {
    let Some(network) = network else {
        return base_network_sandbox_policy;
    };

    match network.enabled {
        Some(true) => NetworkSandboxPolicy::Enabled,
        Some(false) => NetworkSandboxPolicy::Restricted,
        None => base_network_sandbox_policy,
    }
}

fn compile_filesystem_permission(
    path: &str,
    permission: &FilesystemPermissionToml,
    policy_cwd: &Path,
    startup_warnings: &mut Vec<String>,
) -> io::Result<Vec<FileSystemSandboxEntry>> {
    let mut entries = Vec::new();
    match permission {
        FilesystemPermissionToml::Access(access) => {
            entries.push(FileSystemSandboxEntry {
                path: compile_filesystem_access_path(path, *access, startup_warnings)?,
                access: *access,
            });
        }
        FilesystemPermissionToml::Scoped(scoped_entries) => {
            for (subpath, access) in scoped_entries {
                let has_glob = contains_glob_chars(subpath);
                let can_compile_as_pattern = match parse_special_path(path) {
                    Some(FileSystemSpecialPath::ProjectRoots { .. }) | None => true,
                    Some(_) => false,
                };
                if has_glob && *access == FileSystemAccessMode::Deny && can_compile_as_pattern {
                    entries.push(FileSystemSandboxEntry {
                        path: FileSystemPath::GlobPattern {
                            pattern: compile_scoped_filesystem_pattern(path, subpath, *access)?,
                        },
                        access: *access,
                    });
                } else {
                    let subpath = compile_read_write_glob_path(subpath, *access)?;
                    entries.push(FileSystemSandboxEntry {
                        path: compile_scoped_filesystem_path(path, subpath, startup_warnings)?,
                        access: *access,
                    });
                }
            }
        }
    }
    let _ = policy_cwd;
    Ok(entries)
}

fn compile_filesystem_access_path(
    path: &str,
    access: FileSystemAccessMode,
    startup_warnings: &mut Vec<String>,
) -> io::Result<FileSystemPath> {
    if !contains_glob_chars(path) {
        return compile_filesystem_path(path, startup_warnings);
    }

    if access == FileSystemAccessMode::Deny {
        return Ok(FileSystemPath::GlobPattern {
            pattern: parse_absolute_path(path)?.to_string_lossy().into_owned(),
        });
    }

    let path = compile_read_write_glob_path(path, access)?;
    compile_filesystem_path(path, startup_warnings)
}

fn compile_filesystem_path(
    path: &str,
    startup_warnings: &mut Vec<String>,
) -> io::Result<FileSystemPath> {
    if let Some(special) = parse_special_path(path) {
        maybe_push_unknown_special_path_warning(&special, startup_warnings);
        return Ok(FileSystemPath::Special { value: special });
    }

    let path = parse_absolute_path(path)?;
    Ok(FileSystemPath::Path { path })
}

fn compile_scoped_filesystem_path(
    path: &str,
    subpath: &str,
    startup_warnings: &mut Vec<String>,
) -> io::Result<FileSystemPath> {
    if subpath == "." {
        return compile_filesystem_path(path, startup_warnings);
    }

    if let Some(special) = parse_special_path(path) {
        let subpath = parse_relative_subpath(subpath)?;
        let special = match special {
            FileSystemSpecialPath::ProjectRoots { .. } => Ok(FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(Some(subpath)),
            }),
            FileSystemSpecialPath::Unknown { path, .. } => Ok(FileSystemPath::Special {
                value: FileSystemSpecialPath::unknown(path, Some(subpath)),
            }),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("filesystem path `{path}` does not support nested entries"),
            )),
        }?;
        if let FileSystemPath::Special { value } = &special {
            maybe_push_unknown_special_path_warning(value, startup_warnings);
        }
        return Ok(special);
    }

    let subpath = parse_relative_subpath(subpath)?;
    let base = parse_absolute_path(path)?;
    let path = AbsolutePathBuf::resolve_path_against_base(&subpath, base.as_path());
    Ok(FileSystemPath::Path { path })
}

fn compile_scoped_filesystem_pattern(
    path: &str,
    subpath: &str,
    access: FileSystemAccessMode,
) -> io::Result<String> {
    if access != FileSystemAccessMode::Deny {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("filesystem glob subpath `{subpath}` only supports `deny` access"),
        ));
    }
    let subpath = parse_relative_subpath(subpath)?;

    match parse_special_path(path) {
        Some(FileSystemSpecialPath::ProjectRoots { .. }) => {
            Ok(project_roots_glob_pattern(&subpath))
        }
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("filesystem path `{path}` does not support nested entries"),
        )),
        None => {
            let base = parse_absolute_path(path)?;
            Ok(base.join(&subpath).to_string_lossy().to_string())
        }
    }
}

fn compile_read_write_glob_path(path: &str, access: FileSystemAccessMode) -> io::Result<&str> {
    if !contains_glob_chars(path) {
        return Ok(path);
    }

    let path_without_trailing_glob = remove_trailing_glob_suffix(path);
    if !contains_glob_chars(path_without_trailing_glob) {
        return Ok(path_without_trailing_glob);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "filesystem glob path `{path}` only supports `deny` access; use an exact path or trailing `/**` for `{access}` subtree access"
        ),
    ))
}

fn unsupported_read_write_glob_paths(filesystem: &FilesystemPermissionsToml) -> Vec<String> {
    let mut patterns = Vec::new();
    for (path, permission) in &filesystem.entries {
        match permission {
            FilesystemPermissionToml::Access(access) => {
                if *access != FileSystemAccessMode::Deny
                    && contains_glob_chars(remove_trailing_glob_suffix(path))
                {
                    patterns.push(path.clone());
                }
            }
            FilesystemPermissionToml::Scoped(scoped_entries) => {
                for (subpath, access) in scoped_entries {
                    if *access != FileSystemAccessMode::Deny
                        && contains_glob_chars(remove_trailing_glob_suffix(subpath))
                    {
                        patterns.push(format!("{path}/{subpath}"));
                    }
                }
            }
        }
    }
    patterns
}

fn unbounded_unreadable_globstar_paths(filesystem: &FilesystemPermissionsToml) -> Vec<String> {
    if filesystem.glob_scan_max_depth.is_some() {
        return Vec::new();
    }

    let mut patterns = Vec::new();
    for (path, permission) in &filesystem.entries {
        match permission {
            FilesystemPermissionToml::Access(FileSystemAccessMode::Deny) => {
                if path.contains("**") {
                    patterns.push(path.clone());
                }
            }
            FilesystemPermissionToml::Access(_) => {}
            FilesystemPermissionToml::Scoped(scoped_entries) => {
                for (subpath, access) in scoped_entries {
                    if *access == FileSystemAccessMode::Deny && subpath.contains("**") {
                        patterns.push(format!("{path}/{subpath}"));
                    }
                }
            }
        }
    }
    patterns
}

fn validate_glob_scan_max_depth(max_depth: Option<usize>) -> io::Result<Option<usize>> {
    match max_depth {
        Some(0) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "glob_scan_max_depth must be at least 1",
        )),
        _ => Ok(max_depth),
    }
}

fn contains_glob_chars(path: &str) -> bool {
    contains_glob_chars_for_platform(path, cfg!(windows))
}

fn contains_glob_chars_for_platform(path: &str, is_windows: bool) -> bool {
    let normalized_windows_path = if is_windows {
        normalize_windows_device_path(path)
    } else {
        None
    };
    let path = normalized_windows_path.as_deref().unwrap_or(path);
    path.chars().any(|ch| matches!(ch, '*' | '?' | '[' | ']'))
}

fn remove_trailing_glob_suffix(path: &str) -> &str {
    path.strip_suffix("/**").unwrap_or(path)
}

fn parse_special_path(path: &str) -> Option<FileSystemSpecialPath> {
    match path {
        ":root" => Some(FileSystemSpecialPath::Root),
        ":minimal" => Some(FileSystemSpecialPath::Minimal),
        ":workspace_roots" => Some(FileSystemSpecialPath::project_roots(/*subpath*/ None)),
        ":tmpdir" => Some(FileSystemSpecialPath::Tmpdir),
        ":slash_tmp" => Some(FileSystemSpecialPath::SlashTmp),
        _ if path.starts_with(':') => {
            Some(FileSystemSpecialPath::unknown(path, /*subpath*/ None))
        }
        _ => None,
    }
}

fn parse_absolute_path(path: &str) -> io::Result<AbsolutePathBuf> {
    parse_absolute_path_for_platform(path, cfg!(windows))
}

fn parse_absolute_path_for_platform(path: &str, is_windows: bool) -> io::Result<AbsolutePathBuf> {
    let path_ref = normalize_absolute_path_for_platform(path, is_windows);
    if !is_absolute_path_for_platform(path, path_ref.as_ref(), is_windows)
        && path != "~"
        && !path.starts_with("~/")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("filesystem path `{path}` must be absolute, use `~/...`, or start with `:`"),
        ));
    }
    AbsolutePathBuf::from_absolute_path(path_ref.as_ref())
}

fn is_absolute_path_for_platform(path: &str, normalized_path: &Path, is_windows: bool) -> bool {
    if is_windows {
        is_windows_absolute_path(path)
            || is_windows_absolute_path(&normalized_path.to_string_lossy())
    } else {
        normalized_path.is_absolute()
    }
}

fn normalize_absolute_path_for_platform(path: &str, is_windows: bool) -> Cow<'_, Path> {
    if !is_windows {
        return Cow::Borrowed(Path::new(path));
    }

    match normalize_windows_device_path(path) {
        Some(normalized) => Cow::Owned(PathBuf::from(normalized)),
        None => Cow::Borrowed(Path::new(path)),
    }
}

fn normalize_windows_device_path(path: &str) -> Option<String> {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{unc}"));
    }
    if let Some(unc) = path.strip_prefix(r"\\.\UNC\") {
        return Some(format!(r"\\{unc}"));
    }
    if let Some(path) = path.strip_prefix(r"\\?\")
        && is_windows_drive_absolute_path(path)
    {
        return Some(path.to_string());
    }
    if let Some(path) = path.strip_prefix(r"\\.\")
        && is_windows_drive_absolute_path(path)
    {
        return Some(path.to_string());
    }
    None
}

fn is_windows_absolute_path(path: &str) -> bool {
    is_windows_drive_absolute_path(path) || path.starts_with(r"\\")
}

fn is_windows_drive_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn parse_relative_subpath(subpath: &str) -> io::Result<PathBuf> {
    let path = Path::new(subpath);
    if !subpath.is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Ok(path.to_path_buf());
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "filesystem subpath `{}` must be a descendant path without `.` or `..` components",
            path.display()
        ),
    ))
}

fn push_warning(startup_warnings: &mut Vec<String>, message: String) {
    tracing::warn!("{message}");
    startup_warnings.push(message);
}

fn missing_filesystem_entries_warning(profile_name: &str) -> String {
    format!(
        "Permissions profile `{profile_name}` does not define any recognized filesystem entries for this version of Sprite. Filesystem access will remain restricted. Upgrade Sprite if this profile expects filesystem permissions."
    )
}

fn maybe_push_unknown_special_path_warning(
    special: &FileSystemSpecialPath,
    startup_warnings: &mut Vec<String>,
) {
    let FileSystemSpecialPath::Unknown { path, subpath } = special else {
        return;
    };
    push_warning(
        startup_warnings,
        match subpath.as_deref() {
            Some(subpath) => format!(
                "Configured filesystem path `{path}` with nested entry `{}` is not recognized by this version of Sprite and will be ignored. Upgrade Sprite if this path is required.",
                subpath.display()
            ),
            None => format!(
                "Configured filesystem path `{path}` is not recognized by this version of Sprite and will be ignored. Upgrade Sprite if this path is required."
            ),
        },
    );
}
