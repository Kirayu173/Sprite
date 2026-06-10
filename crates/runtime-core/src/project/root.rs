pub use config::default_project_root_markers;
pub use config::project_root_markers_from_config;

#[cfg(test)]
mod tests {
    #[test]
    fn default_project_markers_match_config_defaults() {
        assert_eq!(super::default_project_root_markers(), vec![".git"]);
    }
}
