use crate::AgentJob;
use crate::AgentJobCreateParams;
use crate::AgentJobItem;
use crate::AgentJobItemCreateParams;
use crate::AgentJobItemStatus;
use crate::AgentJobProgress;
use crate::AgentJobStatus;
use crate::GOALS_DB_FILENAME;
use crate::LOGS_DB_FILENAME;
use crate::LogEntry;
use crate::LogQuery;
use crate::LogRow;
use crate::MEMORIES_DB_FILENAME;
use crate::STATE_DB_FILENAME;
use crate::SortKey;
use crate::ThreadMetadata;
use crate::ThreadMetadataBuilder;
use crate::ThreadsPage;
use crate::apply_rollout_item;
use crate::db_diagnostics::DbDiagnostics;
use crate::db_diagnostics::DbKind;
use crate::migrations::runtime_goals_migrator;
use crate::migrations::runtime_logs_migrator;
use crate::migrations::runtime_memories_migrator;
use crate::migrations::runtime_state_migrator;
use crate::model::AgentJobRow;
use crate::model::ThreadRow;
use crate::model::anchor_from_item;
use crate::model::datetime_to_epoch_millis;
use crate::model::datetime_to_epoch_seconds;
use crate::model::epoch_millis_to_datetime;
use crate::paths::file_modified_time_utc;
use chrono::DateTime;
use chrono::Utc;
use log::LevelFilter;
use runtime_protocol::ThreadId;
use runtime_protocol::protocol::RolloutItem;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha384;
use sqlx::ConnectOptions;
use sqlx::QueryBuilder;
use sqlx::Row;
use sqlx::Sqlite;
use sqlx::SqliteConnection;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqliteAutoVacuum;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Duration;
use std::time::Instant;
use tracing::warn;

mod agent_jobs;
mod backfill;
mod goals;
mod logs;
mod memories;
#[cfg(test)]
mod test_support;
mod threads;

pub use goals::GoalAccountingMode;
pub use goals::GoalAccountingOutcome;
pub use goals::GoalStore;
pub use goals::GoalUpdate;
pub use memories::MemoryStore;
pub use threads::ThreadFilterOptions;

// "Partition" is the retained-log-content bucket we cap at 10 MiB:
// - one bucket per non-null thread_id
// - one bucket per threadless (thread_id IS NULL) non-null process_uuid
// - one bucket for threadless rows with process_uuid IS NULL
// This budget tracks each row's persisted rendered log body plus non-body
// metadata, rather than the exact sum of all persisted SQLite column bytes.
const LOG_PARTITION_SIZE_LIMIT_BYTES: i64 = 10 * 1024 * 1024;
const LOG_PARTITION_ROW_LIMIT: i64 = 1_000;

#[derive(Clone, Copy)]
struct RuntimeDbSpec {
    label: &'static str,
    filename: &'static str,
    kind: DbKind,
    open_phase: &'static str,
    migrate_phase: &'static str,
}

impl RuntimeDbSpec {
    fn path(self, sprite_home: &Path) -> PathBuf {
        sprite_home.join(self.filename)
    }
}

const STATE_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "state DB",
    filename: STATE_DB_FILENAME,
    kind: DbKind::State,
    open_phase: "open_state",
    migrate_phase: "migrate_state",
};

const LOGS_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "log DB",
    filename: LOGS_DB_FILENAME,
    kind: DbKind::Logs,
    open_phase: "open_logs",
    migrate_phase: "migrate_logs",
};

const GOALS_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "goals DB",
    filename: GOALS_DB_FILENAME,
    kind: DbKind::Goals,
    open_phase: "open_goals",
    migrate_phase: "migrate_goals",
};

const MEMORIES_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "memories DB",
    filename: MEMORIES_DB_FILENAME,
    kind: DbKind::Memories,
    open_phase: "open_memories",
    migrate_phase: "migrate_memories",
};

const RUNTIME_DBS: [RuntimeDbSpec; 4] = [STATE_DB, LOGS_DB, GOALS_DB, MEMORIES_DB];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDbPath {
    pub label: &'static str,
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct StateRuntime {
    sprite_home: PathBuf,
    default_provider: String,
    pool: Arc<sqlx::SqlitePool>,
    logs_pool: Arc<sqlx::SqlitePool>,
    thread_goals: GoalStore,
    memories: MemoryStore,
    thread_updated_at_millis: Arc<AtomicI64>,
}

impl StateRuntime {
    /// Initialize the state runtime using the provided Sprite home and default provider.
    ///
    /// This opens (and migrates) the SQLite databases under `sprite_home`,
    /// keeping logs in a dedicated file to reduce lock contention with the
    /// rest of the state store.
    pub async fn init(sprite_home: PathBuf, default_provider: String) -> anyhow::Result<Arc<Self>> {
        Self::init_inner(
            sprite_home,
            default_provider,
            /*diagnostics_override*/ None,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn init_with_diagnostics_for_tests(
        sprite_home: PathBuf,
        default_provider: String,
        diagnostics_override: &dyn DbDiagnostics,
    ) -> anyhow::Result<Arc<Self>> {
        Self::init_inner(sprite_home, default_provider, Some(diagnostics_override)).await
    }

    async fn init_inner(
        sprite_home: PathBuf,
        default_provider: String,
        diagnostics_override: Option<&dyn DbDiagnostics>,
    ) -> anyhow::Result<Arc<Self>> {
        tokio::fs::create_dir_all(&sprite_home).await?;
        let state_migrator = runtime_state_migrator();
        let logs_migrator = runtime_logs_migrator();
        let goals_migrator = runtime_goals_migrator();
        let memories_migrator = runtime_memories_migrator();
        let state_path = STATE_DB.path(sprite_home.as_path());
        let logs_path = LOGS_DB.path(sprite_home.as_path());
        let goals_path = GOALS_DB.path(sprite_home.as_path());
        let memories_path = MEMORIES_DB.path(sprite_home.as_path());
        let pool = match open_state_sqlite(&state_path, &state_migrator, diagnostics_override).await
        {
            Ok(db) => Arc::new(db),
            Err(err) => {
                warn!("failed to open state db at {}: {err}", state_path.display());
                return Err(err);
            }
        };
        let logs_pool =
            match open_logs_sqlite(&logs_path, &logs_migrator, diagnostics_override).await {
                Ok(db) => Arc::new(db),
                Err(err) => {
                    warn!("failed to open logs db at {}: {err}", logs_path.display());
                    return Err(err);
                }
            };
        let goals_pool =
            match open_goals_sqlite(&goals_path, &goals_migrator, diagnostics_override).await {
                Ok(db) => Arc::new(db),
                Err(err) => {
                    warn!("failed to open goals db at {}: {err}", goals_path.display());
                    return Err(err);
                }
            };
        let memories_pool =
            match open_memories_sqlite(&memories_path, &memories_migrator, diagnostics_override)
                .await
            {
                Ok(db) => Arc::new(db),
                Err(err) => {
                    warn!(
                        "failed to open memories db at {}: {err}",
                        memories_path.display()
                    );
                    return Err(err);
                }
            };
        let started = Instant::now();
        let backfill_state_result = ensure_backfill_state_row_in_pool(pool.as_ref()).await;
        crate::db_diagnostics::record_init_result(
            diagnostics_override,
            DbKind::State,
            "ensure_backfill_state",
            started.elapsed(),
            &backfill_state_result,
        );
        backfill_state_result?;
        let started = Instant::now();
        let thread_updated_at_millis_result: anyhow::Result<Option<i64>> =
            sqlx::query_scalar("SELECT MAX(threads.updated_at_ms) FROM threads")
                .fetch_one(pool.as_ref())
                .await
                .map_err(anyhow::Error::from);
        crate::db_diagnostics::record_init_result(
            diagnostics_override,
            DbKind::State,
            "post_init_query",
            started.elapsed(),
            &thread_updated_at_millis_result,
        );
        let thread_updated_at_millis = thread_updated_at_millis_result?;
        let thread_updated_at_millis = thread_updated_at_millis.unwrap_or(0);
        let runtime = Arc::new(Self {
            thread_goals: GoalStore::new(Arc::clone(&goals_pool)),
            memories: MemoryStore::new(Arc::clone(&memories_pool), Arc::clone(&pool)),
            pool,
            logs_pool,
            sprite_home,
            default_provider,
            thread_updated_at_millis: Arc::new(AtomicI64::new(thread_updated_at_millis)),
        });
        if let Err(err) = runtime.run_logs_startup_maintenance().await {
            warn!(
                "failed to run startup maintenance for logs db at {}: {err}",
                logs_path.display(),
            );
        }
        Ok(runtime)
    }

    /// Return the configured Sprite home directory for this runtime.
    pub fn sprite_home(&self) -> &Path {
        self.sprite_home.as_path()
    }

    pub fn thread_goals(&self) -> &GoalStore {
        &self.thread_goals
    }

    pub fn memories(&self) -> &MemoryStore {
        &self.memories
    }

    pub async fn clear_memory_data_in_sqlite_home(sqlite_home: &Path) -> anyhow::Result<bool> {
        let memories_path = MEMORIES_DB.path(sqlite_home);
        if !tokio::fs::try_exists(&memories_path).await? {
            return Ok(false);
        }

        let memories_migrator = runtime_memories_migrator();
        let pool = open_memories_sqlite(
            &memories_path,
            &memories_migrator,
            /*diagnostics_override*/ None,
        )
        .await?;
        memories::clear_memory_data_in_pool(&pool).await?;
        pool.close().await;
        Ok(true)
    }
}

fn base_sqlite_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .log_statements(LevelFilter::Off)
}

async fn open_state_sqlite(
    path: &Path,
    migrator: &Migrator,
    diagnostics_override: Option<&dyn DbDiagnostics>,
) -> anyhow::Result<SqlitePool> {
    // New state DBs should use incremental auto-vacuum, but retrofitting an
    // existing DB requires a full VACUUM. Do not attempt that during process
    // startup: it is maintenance work that can contend with foreground writers.
    open_sqlite(path, migrator, STATE_DB, diagnostics_override).await
}

async fn open_logs_sqlite(
    path: &Path,
    migrator: &Migrator,
    diagnostics_override: Option<&dyn DbDiagnostics>,
) -> anyhow::Result<SqlitePool> {
    open_sqlite(path, migrator, LOGS_DB, diagnostics_override).await
}

async fn open_goals_sqlite(
    path: &Path,
    migrator: &Migrator,
    diagnostics_override: Option<&dyn DbDiagnostics>,
) -> anyhow::Result<SqlitePool> {
    open_sqlite(path, migrator, GOALS_DB, diagnostics_override).await
}

async fn open_memories_sqlite(
    path: &Path,
    migrator: &Migrator,
    diagnostics_override: Option<&dyn DbDiagnostics>,
) -> anyhow::Result<SqlitePool> {
    open_sqlite(path, migrator, MEMORIES_DB, diagnostics_override).await
}

async fn open_sqlite(
    path: &Path,
    migrator: &Migrator,
    spec: RuntimeDbSpec,
    diagnostics_override: Option<&dyn DbDiagnostics>,
) -> anyhow::Result<SqlitePool> {
    let options = base_sqlite_options(path).auto_vacuum(SqliteAutoVacuum::Incremental);
    let started = Instant::now();
    let pool_result = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(anyhow::Error::from);
    crate::db_diagnostics::record_init_result(
        diagnostics_override,
        spec.kind,
        spec.open_phase,
        started.elapsed(),
        &pool_result,
    );
    let pool = pool_result?;
    if matches!(spec.kind, DbKind::State) {
        normalize_removed_official_migration_checksums(&pool, migrator).await?;
    }
    let started = Instant::now();
    let migrate_result = migrator.run(&pool).await.map_err(anyhow::Error::from);
    crate::db_diagnostics::record_init_result(
        diagnostics_override,
        spec.kind,
        spec.migrate_phase,
        started.elapsed(),
        &migrate_result,
    );
    migrate_result?;
    Ok(pool)
}

struct RemovedOfficialMigration {
    version: i64,
    description: &'static str,
    original_sql: &'static str,
}

const REMOVED_OFFICIAL_MIGRATIONS: &[RemovedOfficialMigration] = &[
    // These SQL strings are the original upstream migration bodies. They are
    // used only to normalize applied checksums so existing DBs can continue
    // through Sprite's deletion migrations for the removed tables.
    RemovedOfficialMigration {
        version: 24,
        description: "remote control enrollments",
        original_sql: "CREATE TABLE remote_control_enrollments (\n    websocket_url TEXT NOT NULL,\n    account_id TEXT NOT NULL,\n    app_server_client_name TEXT NOT NULL,\n    server_id TEXT NOT NULL,\n    environment_id TEXT NOT NULL,\n    server_name TEXT NOT NULL,\n    updated_at INTEGER NOT NULL,\n    PRIMARY KEY (websocket_url, account_id, app_server_client_name)\n);\n",
    },
    RemovedOfficialMigration {
        version: 28,
        description: "device key bindings",
        original_sql: "CREATE TABLE device_key_bindings (\n    key_id TEXT PRIMARY KEY NOT NULL,\n    account_user_id TEXT NOT NULL,\n    client_id TEXT NOT NULL,\n    created_at INTEGER NOT NULL,\n    updated_at INTEGER NOT NULL\n);\n",
    },
];

async fn normalize_removed_official_migration_checksums(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<()> {
    let migrations_table_exists: i64 = sqlx::query_scalar(
        r#"
SELECT EXISTS(
    SELECT 1
    FROM sqlite_master
    WHERE type = 'table'
      AND name = '_sqlx_migrations'
)
        "#,
    )
    .fetch_one(pool)
    .await?;
    if migrations_table_exists == 0 {
        return Ok(());
    }

    for removed in REMOVED_OFFICIAL_MIGRATIONS {
        let Some(current_migration) = migrator
            .migrations
            .iter()
            .find(|migration| migration.version == removed.version)
        else {
            continue;
        };
        let original_checksum =
            Vec::from(Sha384::digest(removed.original_sql.as_bytes()).as_slice());
        if original_checksum.as_slice() == current_migration.checksum.as_ref() {
            continue;
        }

        sqlx::query(
            r#"
UPDATE _sqlx_migrations
SET checksum = ?
WHERE version = ?
  AND description = ?
  AND success = 1
  AND checksum = ?
            "#,
        )
        .bind(current_migration.checksum.as_ref())
        .bind(removed.version)
        .bind(removed.description)
        .bind(original_checksum)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub(super) async fn ensure_backfill_state_row_in_pool(
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
INSERT INTO backfill_state (id, status, last_watermark, last_success_at, updated_at)
VALUES (?, ?, NULL, NULL, ?)
ON CONFLICT(id) DO NOTHING
            "#,
    )
    .bind(1_i64)
    .bind(crate::BackfillStatus::Pending.as_str())
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

pub fn state_db_filename() -> String {
    STATE_DB.filename.to_string()
}

pub fn state_db_path(sprite_home: &Path) -> PathBuf {
    STATE_DB.path(sprite_home)
}

pub fn logs_db_filename() -> String {
    LOGS_DB.filename.to_string()
}

pub fn logs_db_path(sprite_home: &Path) -> PathBuf {
    LOGS_DB.path(sprite_home)
}

pub fn goals_db_filename() -> String {
    GOALS_DB.filename.to_string()
}

pub fn goals_db_path(sprite_home: &Path) -> PathBuf {
    GOALS_DB.path(sprite_home)
}

pub fn memories_db_filename() -> String {
    MEMORIES_DB.filename.to_string()
}

pub fn memories_db_path(sprite_home: &Path) -> PathBuf {
    MEMORIES_DB.path(sprite_home)
}

pub fn runtime_db_paths(sprite_home: &Path) -> Vec<RuntimeDbPath> {
    RUNTIME_DBS
        .iter()
        .map(|spec| RuntimeDbPath {
            label: spec.label,
            path: spec.path(sprite_home),
        })
        .collect()
}

/// Run SQLite's built-in integrity check against an existing database file.
pub async fn sqlite_integrity_check(path: &Path) -> anyhow::Result<Vec<String>> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .log_statements(LevelFilter::Off);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let rows = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&pool)
        .await?;
    pool.close().await;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::REMOVED_OFFICIAL_MIGRATIONS;
    use super::StateRuntime;
    use super::open_state_sqlite;
    use super::runtime_state_migrator;
    use super::sqlite_integrity_check;
    use super::state_db_path;
    use super::test_support::unique_temp_dir;
    use crate::DB_INIT_METRIC;
    use crate::DbDiagnostics;
    use crate::migrations::STATE_MIGRATOR;
    use pretty_assertions::assert_eq;
    use sha2::Digest;
    use sha2::Sha384;
    use sqlx::SqlitePool;
    use sqlx::migrate::MigrateError;
    use sqlx::migrate::Migrator;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::borrow::Cow;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestDiagnostics {
        counters: Mutex<Vec<MetricEvent>>,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct MetricEvent {
        name: String,
        tags: BTreeMap<String, String>,
    }

    impl TestDiagnostics {
        fn counters(&self) -> Vec<MetricEvent> {
            self.counters
                .lock()
                .expect("diagnostics lock")
                .iter()
                .map(|event| MetricEvent {
                    name: event.name.clone(),
                    tags: event.tags.clone(),
                })
                .collect()
        }
    }

    impl DbDiagnostics for TestDiagnostics {
        fn counter(&self, name: &str, _inc: i64, tags: &[(&str, &str)]) {
            self.counters
                .lock()
                .expect("diagnostics lock")
                .push(MetricEvent {
                    name: name.to_string(),
                    tags: tags_to_map(tags),
                });
        }

        fn record_duration(
            &self,
            _name: &str,
            _duration: std::time::Duration,
            _tags: &[(&str, &str)],
        ) {
        }
    }

    fn tags_to_map(tags: &[(&str, &str)]) -> BTreeMap<String, String> {
        tags.iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    async fn open_db_pool(path: &Path) -> SqlitePool {
        SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false),
        )
        .await
        .expect("open sqlite pool")
    }

    #[tokio::test]
    async fn sqlite_integrity_check_reports_ok_for_valid_db() {
        let sprite_home = unique_temp_dir();
        tokio::fs::create_dir_all(&sprite_home)
            .await
            .expect("create sprite home");
        let path = state_db_path(sprite_home.as_path());
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .expect("open sqlite db");
        sqlx::query("CREATE TABLE sample (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create sample table");
        pool.close().await;

        let result = sqlite_integrity_check(&path)
            .await
            .expect("integrity check should run");

        assert_eq!(result, vec!["ok".to_string()]);
        let _ = tokio::fs::remove_dir_all(sprite_home).await;
    }

    #[tokio::test]
    async fn open_state_sqlite_tolerates_newer_applied_migrations() {
        let sprite_home = unique_temp_dir();
        tokio::fs::create_dir_all(&sprite_home)
            .await
            .expect("create sprite home");
        let state_path = state_db_path(sprite_home.as_path());
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&state_path)
                .create_if_missing(true),
        )
        .await
        .expect("open state db");
        STATE_MIGRATOR
            .run(&pool)
            .await
            .expect("apply current state schema");
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(9_999_i64)
        .bind("future migration")
        .bind(true)
        .bind(vec![1_u8, 2, 3, 4])
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("insert future migration record");
        pool.close().await;

        let strict_pool = open_db_pool(state_path.as_path()).await;
        let strict_err = STATE_MIGRATOR
            .run(&strict_pool)
            .await
            .expect_err("strict migrator should reject newer applied migrations");
        assert!(matches!(strict_err, MigrateError::VersionMissing(9_999)));
        strict_pool.close().await;

        let tolerant_migrator = runtime_state_migrator();
        let tolerant_pool = open_state_sqlite(
            state_path.as_path(),
            &tolerant_migrator,
            /*diagnostics_override*/ None,
        )
        .await
        .expect("runtime migrator should tolerate newer applied migrations");
        tolerant_pool.close().await;

        let _ = tokio::fs::remove_dir_all(sprite_home).await;
    }

    #[tokio::test]
    async fn open_state_sqlite_normalizes_removed_official_migration_checksums() {
        let sprite_home = unique_temp_dir();
        tokio::fs::create_dir_all(&sprite_home)
            .await
            .expect("create sprite home");
        let state_path = state_db_path(sprite_home.as_path());
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&state_path)
                .create_if_missing(true),
        )
        .await
        .expect("open state db");
        let pre_official_deletion_migrator = Migrator {
            migrations: Cow::Owned(
                STATE_MIGRATOR
                    .migrations
                    .iter()
                    .filter(|migration| migration.version < 24)
                    .cloned()
                    .collect(),
            ),
            ignore_missing: false,
            locking: STATE_MIGRATOR.locking,
            no_tx: STATE_MIGRATOR.no_tx,
        };
        pre_official_deletion_migrator
            .run(&pool)
            .await
            .expect("apply schema before official migrations");
        sqlx::query(
            "CREATE TABLE remote_control_enrollments (
                websocket_url TEXT NOT NULL,
                account_id TEXT NOT NULL,
                app_server_client_name TEXT NOT NULL,
                server_id TEXT NOT NULL,
                environment_id TEXT NOT NULL,
                server_name TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (websocket_url, account_id, app_server_client_name)
            )",
        )
        .execute(&pool)
        .await
        .expect("create upstream remote control table");
        sqlx::query(
            "CREATE TABLE device_key_bindings (
                key_id TEXT PRIMARY KEY NOT NULL,
                account_user_id TEXT NOT NULL,
                client_id TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create upstream device key table");
        for removed in REMOVED_OFFICIAL_MIGRATIONS {
            let original_checksum =
                Vec::from(Sha384::digest(removed.original_sql.as_bytes()).as_slice());
            sqlx::query(
                "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (?, ?, ?, ?, ?)",
            )
                .bind(removed.version)
                .bind(removed.description)
                .bind(true)
                .bind(original_checksum)
                .bind(1_i64)
                .execute(&pool)
                .await
                .expect("insert upstream migration checksum");
        }
        pool.close().await;

        let strict_pool = open_db_pool(state_path.as_path()).await;
        let strict_err = STATE_MIGRATOR
            .run(&strict_pool)
            .await
            .expect_err("strict migrator should reject removed official migration checksum");
        assert!(matches!(strict_err, MigrateError::VersionMismatch(24)));
        strict_pool.close().await;

        let tolerant_migrator = runtime_state_migrator();
        let tolerant_pool = open_state_sqlite(
            state_path.as_path(),
            &tolerant_migrator,
            /*diagnostics_override*/ None,
        )
        .await
        .expect("runtime migrator should normalize removed official checksums");
        let remote_table_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'remote_control_enrollments'",
        )
        .fetch_one(&tolerant_pool)
        .await
        .expect("query remote control table");
        let device_table_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'device_key_bindings'",
        )
        .fetch_one(&tolerant_pool)
        .await
        .expect("query device key table");
        assert_eq!(remote_table_exists, 0);
        assert_eq!(device_table_exists, 0);
        tolerant_pool.close().await;

        let _ = tokio::fs::remove_dir_all(sprite_home).await;
    }

    #[tokio::test]
    async fn init_records_successful_sqlite_init_phases_to_explicit_diagnostics() {
        let sprite_home = unique_temp_dir();
        let diagnostics = TestDiagnostics::default();

        let runtime = StateRuntime::init_with_diagnostics_for_tests(
            sprite_home.clone(),
            "test-provider".to_string(),
            &diagnostics,
        )
        .await
        .expect("state runtime should initialize");

        let phases = diagnostics
            .counters()
            .into_iter()
            .filter(|event| event.name == DB_INIT_METRIC)
            .filter(|event| event.tags.get("status").map(String::as_str) == Some("success"))
            .filter_map(|event| event.tags.get("phase").cloned())
            .collect::<BTreeSet<_>>();
        let expected = [
            "open_state",
            "migrate_state",
            "open_logs",
            "migrate_logs",
            "open_goals",
            "migrate_goals",
            "open_memories",
            "migrate_memories",
            "ensure_backfill_state",
            "post_init_query",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
        assert_eq!(phases, expected);

        runtime.pool.close().await;
        runtime.logs_pool.close().await;
        let _ = tokio::fs::remove_dir_all(sprite_home).await;
    }
}
