use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use sha1::digest::Output;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";
const DEV_NULL: &str = "/dev/null";
const REGULAR_FILE_MODE: &str = "100644";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchDelta {
    exact: bool,
    changes: Vec<PatchChange>,
}

impl PatchDelta {
    pub fn exact(changes: Vec<PatchChange>) -> Self {
        Self {
            exact: true,
            changes,
        }
    }

    pub fn inexact(changes: Vec<PatchChange>) -> Self {
        Self {
            exact: false,
            changes,
        }
    }

    pub fn is_exact(&self) -> bool {
        self.exact
    }

    pub fn changes(&self) -> &[PatchChange] {
        &self.changes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchChange {
    pub path: PathBuf,
    pub change: PatchFileChange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchFileChange {
    Add {
        content: String,
        overwritten_content: Option<String>,
    },
    Delete {
        content: String,
    },
    Update {
        move_path: Option<PathBuf>,
        old_content: String,
        overwritten_move_content: Option<String>,
        new_content: String,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TrackedPath {
    environment_id: String,
    path: PathBuf,
}

impl TrackedPath {
    fn new(environment_id: &str, path: &Path) -> Self {
        Self {
            environment_id: environment_id.to_string(),
            path: path.to_path_buf(),
        }
    }
}

/// Tracks the net text diff for the current turn from committed patch deltas.
///
/// This mirrors the upstream apply-patch tracker shape, but keeps a local delta
/// boundary until the apply-patch crate is migrated into the Sprite workspace.
pub struct TurnDiffTracker {
    valid: bool,
    display_roots_by_environment: HashMap<String, PathBuf>,
    baseline_by_path: HashMap<TrackedPath, String>,
    current_by_path: HashMap<TrackedPath, String>,
    origin_by_current_path: HashMap<TrackedPath, TrackedPath>,
}

impl Default for TurnDiffTracker {
    fn default() -> Self {
        Self {
            valid: true,
            display_roots_by_environment: HashMap::new(),
            baseline_by_path: HashMap::new(),
            current_by_path: HashMap::new(),
            origin_by_current_path: HashMap::new(),
        }
    }
}

impl TurnDiffTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_environment_display_roots(
        display_roots: impl IntoIterator<Item = (String, PathBuf)>,
    ) -> Self {
        let mut tracker = Self::new();
        tracker.display_roots_by_environment = display_roots.into_iter().collect();
        tracker
    }

    pub fn track_delta(&mut self, environment_id: &str, delta: &PatchDelta) {
        if !delta.is_exact() {
            self.invalidate();
            return;
        }

        for change in delta.changes() {
            self.apply_change(environment_id, change);
        }
    }

    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    pub fn get_unified_diff(&self) -> Option<String> {
        if !self.valid {
            return None;
        }

        let rename_pairs = self.rename_pairs();
        let paired_destinations = rename_pairs.values().cloned().collect::<HashSet<_>>();
        let mut handled = HashSet::new();
        let mut paths = self
            .baseline_by_path
            .keys()
            .chain(self.current_by_path.keys())
            .cloned()
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| self.display_path(path));
        paths.dedup();

        let mut aggregated = String::new();
        for path in paths {
            if !handled.insert(path.clone()) {
                continue;
            }

            if paired_destinations.contains(&path) {
                continue;
            }

            let diff = if let Some(dest) = rename_pairs.get(&path) {
                handled.insert(dest.clone());
                self.render_rename_diff(&path, dest)
            } else {
                self.render_path_diff(&path)
            };

            if let Some(diff) = diff {
                aggregated.push_str(&diff);
                if !aggregated.ends_with('\n') {
                    aggregated.push('\n');
                }
            }
        }

        (!aggregated.is_empty()).then_some(aggregated)
    }

    fn apply_change(&mut self, environment_id: &str, change: &PatchChange) {
        let source_path = TrackedPath::new(environment_id, change.path.as_path());
        match &change.change {
            PatchFileChange::Add {
                content,
                overwritten_content,
            } => self.apply_add(source_path, content, overwritten_content.as_deref()),
            PatchFileChange::Delete { content } => self.apply_delete(source_path, content),
            PatchFileChange::Update {
                move_path,
                old_content,
                overwritten_move_content,
                new_content,
            } => {
                let move_path = move_path
                    .as_deref()
                    .map(|path| TrackedPath::new(environment_id, path));
                self.apply_update(
                    source_path,
                    move_path,
                    old_content,
                    overwritten_move_content.as_deref(),
                    new_content,
                )
            }
        }
    }

    fn apply_add(&mut self, path: TrackedPath, content: &str, overwritten_content: Option<&str>) {
        self.origin_by_current_path.remove(&path);
        if !self.current_by_path.contains_key(&path)
            && !self.baseline_by_path.contains_key(&path)
            && let Some(overwritten_content) = overwritten_content
        {
            self.baseline_by_path
                .insert(path.clone(), overwritten_content.to_string());
        }
        self.current_by_path.insert(path, content.to_string());
    }

    fn apply_delete(&mut self, path: TrackedPath, content: &str) {
        if self.current_by_path.remove(&path).is_none() && !self.baseline_by_path.contains_key(&path)
        {
            self.baseline_by_path
                .insert(path.clone(), content.to_string());
        }
        self.origin_by_current_path.remove(&path);
    }

    fn apply_update(
        &mut self,
        source_path: TrackedPath,
        move_path: Option<TrackedPath>,
        old_content: &str,
        overwritten_move_content: Option<&str>,
        new_content: &str,
    ) {
        if !self.current_by_path.contains_key(&source_path)
            && !self.baseline_by_path.contains_key(&source_path)
        {
            self.baseline_by_path
                .insert(source_path.clone(), old_content.to_string());
        }

        match move_path {
            Some(dest_path) => {
                if !self.current_by_path.contains_key(&dest_path)
                    && !self.baseline_by_path.contains_key(&dest_path)
                    && let Some(overwritten_move_content) = overwritten_move_content
                {
                    self.baseline_by_path
                        .insert(dest_path.clone(), overwritten_move_content.to_string());
                }
                let origin = self
                    .origin_by_current_path
                    .remove(&source_path)
                    .unwrap_or_else(|| source_path.clone());
                self.current_by_path.remove(&source_path);
                self.current_by_path
                    .insert(dest_path.clone(), new_content.to_string());
                self.origin_by_current_path.remove(&dest_path);
                if dest_path != origin {
                    self.origin_by_current_path.insert(dest_path, origin);
                }
            }
            None => {
                self.current_by_path.insert(source_path, new_content.to_string());
            }
        }
    }

    fn rename_pairs(&self) -> HashMap<TrackedPath, TrackedPath> {
        self.origin_by_current_path
            .iter()
            .filter_map(|(dest_path, origin_path)| {
                if dest_path == origin_path
                    || self.current_by_path.contains_key(origin_path)
                    || !self.current_by_path.contains_key(dest_path)
                    || !self.baseline_by_path.contains_key(origin_path)
                    || self.baseline_by_path.contains_key(dest_path)
                {
                    return None;
                }

                Some((origin_path.clone(), dest_path.clone()))
            })
            .collect()
    }

    fn render_path_diff(&self, path: &TrackedPath) -> Option<String> {
        self.render_diff(
            path,
            self.baseline_by_path.get(path).map(String::as_str),
            path,
            self.current_by_path.get(path).map(String::as_str),
        )
    }

    fn render_rename_diff(
        &self,
        source_path: &TrackedPath,
        dest_path: &TrackedPath,
    ) -> Option<String> {
        self.render_diff(
            source_path,
            self.baseline_by_path.get(source_path).map(String::as_str),
            dest_path,
            self.current_by_path.get(dest_path).map(String::as_str),
        )
    }

    fn render_diff(
        &self,
        left_path: &TrackedPath,
        left_content: Option<&str>,
        right_path: &TrackedPath,
        right_content: Option<&str>,
    ) -> Option<String> {
        if left_content == right_content {
            return None;
        }

        let left_display = self.display_path(left_path);
        let right_display = self.display_path(right_path);
        let left_oid = left_content.map_or_else(
            || ZERO_OID.to_string(),
            |content| git_blob_oid(content.as_bytes()),
        );
        let right_oid = right_content.map_or_else(
            || ZERO_OID.to_string(),
            |content| git_blob_oid(content.as_bytes()),
        );

        let mut diff = format!("diff --git a/{left_display} b/{right_display}\n");
        match (left_content, right_content) {
            (None, Some(_)) => diff.push_str(&format!("new file mode {REGULAR_FILE_MODE}\n")),
            (Some(_), None) => diff.push_str(&format!("deleted file mode {REGULAR_FILE_MODE}\n")),
            (Some(_), Some(_)) => {}
            (None, None) => return None,
        }

        diff.push_str(&format!("index {left_oid}..{right_oid}\n"));

        let old_header = if left_content.is_some() {
            format!("a/{left_display}")
        } else {
            DEV_NULL.to_string()
        };
        let new_header = if right_content.is_some() {
            format!("b/{right_display}")
        } else {
            DEV_NULL.to_string()
        };

        let unified =
            similar::TextDiff::from_lines(left_content.unwrap_or(""), right_content.unwrap_or(""))
                .unified_diff()
                .context_radius(3)
                .header(&old_header, &new_header)
                .to_string();
        diff.push_str(&unified);
        Some(diff)
    }

    fn display_path(&self, path: &TrackedPath) -> String {
        let display = self
            .display_roots_by_environment
            .get(&path.environment_id)
            .and_then(|root| path.path.strip_prefix(root).ok())
            .unwrap_or(path.path.as_path());
        let display = display.display().to_string().replace('\\', "/");
        if self.display_roots_by_environment.len() > 1 && !path.environment_id.is_empty() {
            format!("{}/{display}", path.environment_id)
        } else {
            display
        }
    }
}

fn git_blob_oid(data: &[u8]) -> String {
    format!("{:x}", git_blob_sha1_hex_bytes(data))
}

fn git_blob_sha1_hex_bytes(data: &[u8]) -> Output<sha1::Sha1> {
    let header = format!("blob {}\0", data.len());
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(data);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::DEV_NULL;
    use super::PatchChange;
    use super::PatchDelta;
    use super::PatchFileChange;
    use super::REGULAR_FILE_MODE;
    use super::TurnDiffTracker;
    use super::ZERO_OID;
    use super::git_blob_oid;

    fn tracker_with_root(root: &Path) -> TurnDiffTracker {
        TurnDiffTracker::with_environment_display_roots([("".to_string(), root.to_path_buf())])
    }

    #[test]
    fn accumulates_add_then_update_as_single_add() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tracker = tracker_with_root(dir.path());

        tracker.track_delta(
            "",
            &PatchDelta::exact(vec![PatchChange {
                path: dir.path().join("a.txt"),
                change: PatchFileChange::Add {
                    content: "foo\n".to_string(),
                    overwritten_content: None,
                },
            }]),
        );

        tracker.track_delta(
            "",
            &PatchDelta::exact(vec![PatchChange {
                path: dir.path().join("a.txt"),
                change: PatchFileChange::Update {
                    move_path: None,
                    old_content: "foo\n".to_string(),
                    overwritten_move_content: None,
                    new_content: "foo\nbar\n".to_string(),
                },
            }]),
        );

        let right_oid = git_blob_oid("foo\nbar\n".as_bytes());
        let expected = format!(
            r#"diff --git a/a.txt b/a.txt
new file mode {REGULAR_FILE_MODE}
index {ZERO_OID}..{right_oid}
--- {DEV_NULL}
+++ b/a.txt
@@ -0,0 +1,2 @@
+foo
+bar
"#,
        );
        assert_eq!(tracker.get_unified_diff(), Some(expected));
    }

    #[test]
    fn invalidated_tracker_suppresses_existing_diff() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tracker = tracker_with_root(dir.path());
        tracker.track_delta(
            "",
            &PatchDelta::exact(vec![PatchChange {
                path: dir.path().join("a.txt"),
                change: PatchFileChange::Add {
                    content: "foo\n".to_string(),
                    overwritten_content: None,
                },
            }]),
        );

        tracker.invalidate();

        assert_eq!(tracker.get_unified_diff(), None);
    }

    #[test]
    fn tracks_same_absolute_path_across_multiple_environments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let change = PatchChange {
            path: dir.path().join("shared.txt"),
            change: PatchFileChange::Add {
                content: "content\n".to_string(),
                overwritten_content: None,
            },
        };

        let mut tracker = TurnDiffTracker::with_environment_display_roots([
            ("local".to_string(), dir.path().to_path_buf()),
            ("remote".to_string(), dir.path().to_path_buf()),
        ]);
        tracker.track_delta("remote", &PatchDelta::exact(vec![change.clone()]));
        tracker.track_delta("local", &PatchDelta::exact(vec![change]));

        let right_oid = git_blob_oid("content\n".as_bytes());
        let expected = format!(
            r#"diff --git a/local/shared.txt b/local/shared.txt
new file mode {REGULAR_FILE_MODE}
index {ZERO_OID}..{right_oid}
--- {DEV_NULL}
+++ b/local/shared.txt
@@ -0,0 +1 @@
+content
diff --git a/remote/shared.txt b/remote/shared.txt
new file mode {REGULAR_FILE_MODE}
index {ZERO_OID}..{right_oid}
--- {DEV_NULL}
+++ b/remote/shared.txt
@@ -0,0 +1 @@
+content
"#,
        );
        assert_eq!(tracker.get_unified_diff(), Some(expected));
    }

    #[test]
    fn accumulates_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tracker = tracker_with_root(dir.path());
        tracker.track_delta(
            "",
            &PatchDelta::exact(vec![PatchChange {
                path: dir.path().join("b.txt"),
                change: PatchFileChange::Delete {
                    content: "x\n".to_string(),
                },
            }]),
        );

        let left_oid = git_blob_oid("x\n".as_bytes());
        let expected = format!(
            r#"diff --git a/b.txt b/b.txt
deleted file mode {REGULAR_FILE_MODE}
index {left_oid}..{ZERO_OID}
--- a/b.txt
+++ {DEV_NULL}
@@ -1 +0,0 @@
-x
"#,
        );
        assert_eq!(tracker.get_unified_diff(), Some(expected));
    }

    #[test]
    fn accumulates_move_and_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tracker = tracker_with_root(dir.path());
        tracker.track_delta(
            "",
            &PatchDelta::exact(vec![PatchChange {
                path: dir.path().join("src.txt"),
                change: PatchFileChange::Update {
                    move_path: Some(dir.path().join("dst.txt")),
                    old_content: "line\n".to_string(),
                    overwritten_move_content: None,
                    new_content: "line2\n".to_string(),
                },
            }]),
        );

        let left_oid = git_blob_oid("line\n".as_bytes());
        let right_oid = git_blob_oid("line2\n".as_bytes());
        let expected = format!(
            r#"diff --git a/src.txt b/dst.txt
index {left_oid}..{right_oid}
--- a/src.txt
+++ b/dst.txt
@@ -1 +1 @@
-line
+line2
"#,
        );
        assert_eq!(tracker.get_unified_diff(), Some(expected));
    }

    #[test]
    fn pure_rename_yields_no_diff() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tracker = tracker_with_root(dir.path());
        tracker.track_delta(
            "",
            &PatchDelta::exact(vec![PatchChange {
                path: dir.path().join("old.txt"),
                change: PatchFileChange::Update {
                    move_path: Some(dir.path().join("new.txt")),
                    old_content: "same\n".to_string(),
                    overwritten_move_content: None,
                    new_content: "same\n".to_string(),
                },
            }]),
        );

        assert_eq!(tracker.get_unified_diff(), None);
    }

    #[test]
    fn inexact_delta_invalidates_tracker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut tracker = tracker_with_root(dir.path());
        tracker.track_delta(
            "",
            &PatchDelta::inexact(vec![PatchChange {
                path: dir.path().join("ignored.txt"),
                change: PatchFileChange::Add {
                    content: "ignored\n".to_string(),
                    overwritten_content: None,
                },
            }]),
        );

        assert_eq!(tracker.get_unified_diff(), None);
    }
}
