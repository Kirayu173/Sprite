//! Rollout persistence and discovery for Sprite session files.

use std::sync::LazyLock;

use runtime_protocol::protocol::SessionSource;

pub(crate) mod compression;
pub(crate) mod config;
pub(crate) mod list;
pub(crate) mod metadata;
pub(crate) mod policy;
pub(crate) mod reconstruction;
pub(crate) mod recorder;
pub(crate) mod search;
pub(crate) mod session_index;
mod sqlite_metrics;
pub mod state_db;
pub(crate) mod truncation;

pub(crate) mod local_client {
    pub(crate) struct Originator {
        pub(crate) value: String,
    }

    pub(crate) fn originator() -> Originator {
        Originator {
            value: "sprite".to_string(),
        }
    }
}

pub(crate) use runtime_protocol::protocol;

pub const SESSIONS_SUBDIR: &str = "sessions";
pub const ARCHIVED_SESSIONS_SUBDIR: &str = "archived_sessions";
pub static INTERACTIVE_SESSION_SOURCES: LazyLock<Vec<SessionSource>> = LazyLock::new(|| {
    vec![
        SessionSource::Cli,
        SessionSource::VSCode,
        SessionSource::Exec,
        SessionSource::Mcp,
    ]
});

pub use compression::RolloutLineReader;
pub use compression::existing_rollout_path;
pub use compression::open_rollout_line_reader;
pub use compression::plain_rollout_path;
pub use compression::spawn_rollout_compression_worker;
pub use config::Config;
pub use config::RolloutConfig;
pub use config::RolloutConfigView;
pub use list::Cursor;
pub use list::SortDirection;
pub use list::ThreadItem;
pub use list::ThreadListConfig;
pub use list::ThreadListLayout;
pub use list::ThreadSortKey;
pub use list::ThreadsPage;
pub use list::find_archived_thread_path_by_id_str;
pub use list::find_thread_path_by_id_str;
#[deprecated(note = "use find_thread_path_by_id_str")]
pub use list::find_thread_path_by_id_str as find_conversation_path_by_id_str;
pub use list::get_threads;
pub use list::get_threads_in_root;
pub use list::parse_cursor;
pub use list::read_head_for_summary;
pub use list::read_session_meta_line;
pub use list::read_thread_item_from_rollout;
pub use list::rollout_date_parts;
pub use metadata::builder_from_items;
pub use policy::is_persisted_rollout_item;
pub use policy::persisted_rollout_items;
pub use policy::should_persist_response_item_for_memories;
pub use reconstruction::PreviousTurnSettings;
pub use reconstruction::RolloutReconstruction;
pub use reconstruction::reconstruct_history_from_rollout;
pub use recorder::RolloutRecorder;
pub use recorder::RolloutRecorderParams;
pub use recorder::append_rollout_item_to_path;
pub use runtime_protocol::protocol::SessionMeta;
pub use search::first_rollout_content_match_snippet;
pub use search::search_rollout_matches;
pub use search::search_rollout_paths;
pub use session_index::append_thread_name;
pub use session_index::find_thread_meta_by_name_str;
pub use session_index::find_thread_name_by_id;
pub use session_index::find_thread_names_by_ids;
pub use state_db::StateDbHandle;
pub use state_db::sqlite_diagnostics_recorder;
pub use truncation::fork_turn_positions_in_rollout;
pub use truncation::initial_history_has_prior_user_turns;
pub use truncation::is_user_turn_boundary;
pub use truncation::truncate_rollout_before_nth_user_message_from_start;
pub use truncation::truncate_rollout_to_last_n_fork_turns;
pub use truncation::user_message_positions_in_rollout;

#[cfg(test)]
mod tests;
