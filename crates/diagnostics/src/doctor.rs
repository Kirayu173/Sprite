use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CheckStatus {
    Ok,
    Warning,
    Fail,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorCheck {
    pub id: String,
    pub category: String,
    pub status: CheckStatus,
    pub summary: String,
    pub details: Vec<String>,
}

impl DoctorCheck {
    pub fn new(
        id: impl Into<String>,
        category: impl Into<String>,
        status: CheckStatus,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status,
            summary: summary.into(),
            details: Vec::new(),
        }
    }

    pub fn details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }
}

pub fn runtime_check(service_version: &str, build_commit: Option<&str>) -> DoctorCheck {
    let current_exe = std::env::current_exe().ok();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let mut details = vec![
        format!("version: {service_version}"),
        format!("platform: {platform}"),
        format!("commit: {}", build_commit.unwrap_or("unknown")),
    ];
    push_path_detail(&mut details, "current executable", current_exe.as_deref());

    DoctorCheck::new(
        "runtime.provenance",
        "runtime",
        CheckStatus::Ok,
        format!("running on {platform}"),
    )
    .details(details)
}

pub fn system_check() -> DoctorCheck {
    let info = os_info::get();
    let locale_env = locale_env();
    let mut details = vec![
        format!("os: {info}"),
        format!("os type: {}", info.os_type()),
        format!("os version: {}", info.version()),
    ];
    match sys_locale::get_locale() {
        Some(language) => details.push(format!("os language: {language}")),
        None => details.push("os language: unavailable".to_string()),
    }
    for (name, value) in locale_env {
        details.push(format!("{name}: {value}"));
    }

    DoctorCheck::new(
        "system.environment",
        "system",
        CheckStatus::Ok,
        "system environment detected",
    )
    .details(details)
}

pub fn git_check(cwd: &Path) -> DoctorCheck {
    let selected_git = which::which("git").ok();
    let git_candidates = git_candidates();
    let mut details = Vec::new();
    match selected_git.as_deref() {
        Some(path) => details.push(format!("selected git: {}", path.display())),
        None => details.push("selected git: not found".to_string()),
    }
    details.push(format!("PATH git entries: {}", git_candidates.len()));
    for (index, path) in git_candidates.iter().enumerate() {
        details.push(format!("PATH git #{}: {}", index + 1, path.display()));
    }

    let version = selected_git
        .as_deref()
        .and_then(|git| command_output(git, cwd, &["--version"]));
    if let Some(version) = version.as_deref() {
        details.push(format!("git version: {version}"));
    }

    let status = if selected_git.is_some() && version.is_none() {
        CheckStatus::Warning
    } else {
        CheckStatus::Ok
    };
    let summary =
        version.unwrap_or_else(|| "git executable not found or version unavailable".to_string());

    DoctorCheck::new("git.environment", "git", status, summary).details(details)
}

fn push_path_detail(details: &mut Vec<String>, label: &str, path: Option<&Path>) {
    match path {
        Some(path) => details.push(format!("{label}: {}", path.display())),
        None => details.push(format!("{label}: unavailable")),
    }
}

fn locale_env() -> BTreeMap<String, String> {
    ["LANG", "LC_ALL", "LC_MESSAGES", "LC_CTYPE"]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect()
}

fn git_candidates() -> Vec<PathBuf> {
    let Ok(candidates) = which::which_all("git") else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    candidates
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

fn command_output(program: &Path, cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let normalized = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_check_includes_version_and_platform() {
        let check = runtime_check("0.1.0", Some("abc"));

        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.details.contains(&"version: 0.1.0".to_string()));
        assert!(check.details.contains(&"commit: abc".to_string()));
    }
}
