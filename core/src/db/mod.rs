//! SQLite persistence: connection pool, embedded migrations, and repositories.
//!
//! Schema v1 lives in `core/migrations/`. Repositories are added alongside their
//! consumers (settings, jobs; runtimes in WP-4, models with the Phase-2 model
//! importer) rather than all up front.

mod agents;
mod bench;
mod documents;
mod downloads;
mod jobs;
mod models;
mod runtimes;
mod sessions;
mod settings;
pub mod stories;
mod voice_identities;

pub use agents::{Agent, AgentRepo, AgentSession, AgentSessionEvent, AgentSessionState, NewAgent};
pub use bench::{BenchRepo, Benchmark, NewBenchmark};
pub use documents::{Document, DocumentChunk, DocumentRepo, NewDocument};
pub use downloads::{Download, DownloadRepo, DownloadState, NewDownload};
pub use jobs::{EventLevel, Job, JobEvent, JobFilter, JobPatch, JobRepo, NewJob};
pub use models::{Model, ModelLink, ModelRepo, NewModel};
pub use runtimes::{state as runtime_state, RuntimeRecord, RuntimeRepo};
pub use sessions::{Session, SessionRepo};
pub use settings::SettingsRepo;
pub use stories::{
    Character, CharacterLogEntry, CharacterRelationship, CharacterRepo, CharacterUpdate,
    DialogueLine, Location, LocationRepo, LocationUpdate, NewCharacter, NewDialogueLine,
    NewLocation, NewNpc, NewScene, Npc, NpcRepo, NpcUpdate, Scene, SceneImage, SceneImageRepo,
    SceneRepo, SceneUpdate, Story, StoryRepo, StoryUpdate,
};
pub use voice_identities::{VoiceIdentity, VoiceIdentityRepo};

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{CoreError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A live database handle. Cloning is cheap (the pool is reference-counted).
#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open (creating if needed) the on-disk database at `file` and run pending
    /// migrations.
    pub async fn connect(file: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(file)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;
        Self::migrated(pool).await
    }

    /// A private in-memory database for tests. The single connection is pinned so
    /// the schema survives for the lifetime of the returned handle.
    pub async fn connect_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await?;
        Self::migrated(pool).await
    }

    async fn migrated(pool: SqlitePool) -> Result<Self> {
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn settings(&self) -> SettingsRepo<'_> {
        SettingsRepo::new(&self.pool)
    }

    pub fn jobs(&self) -> JobRepo<'_> {
        JobRepo::new(&self.pool)
    }

    pub fn models(&self) -> ModelRepo<'_> {
        ModelRepo::new(&self.pool)
    }

    pub fn runtimes(&self) -> RuntimeRepo<'_> {
        RuntimeRepo::new(&self.pool)
    }

    pub fn agents(&self) -> AgentRepo<'_> {
        AgentRepo::new(&self.pool)
    }

    pub fn sessions(&self) -> SessionRepo<'_> {
        SessionRepo::new(&self.pool)
    }

    pub fn documents(&self) -> DocumentRepo<'_> {
        DocumentRepo::new(&self.pool)
    }

    pub fn downloads(&self) -> DownloadRepo<'_> {
        DownloadRepo::new(&self.pool)
    }

    pub fn benchmarks(&self) -> BenchRepo<'_> {
        BenchRepo::new(&self.pool)
    }

    pub fn voice_identities(&self) -> VoiceIdentityRepo<'_> {
        VoiceIdentityRepo::new(&self.pool)
    }

    pub fn stories(&self) -> StoryRepo<'_> {
        StoryRepo::new(&self.pool)
    }

    pub fn characters(&self) -> CharacterRepo<'_> {
        CharacterRepo::new(&self.pool)
    }

    pub fn npcs(&self) -> NpcRepo<'_> {
        NpcRepo::new(&self.pool)
    }

    pub fn locations(&self) -> LocationRepo<'_> {
        LocationRepo::new(&self.pool)
    }

    pub fn scenes(&self) -> SceneRepo<'_> {
        SceneRepo::new(&self.pool)
    }

    pub fn scene_images(&self) -> SceneImageRepo<'_> {
        SceneImageRepo::new(&self.pool)
    }

    /// Names of the application tables (excludes SQLite internals). Test helper.
    pub async fn table_names(&self) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '\\_%' ESCAPE '\\'
             ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    /// Write a consistent snapshot of the whole database to `dest` (`VACUUM
    /// INTO` — a single clean file, WAL folded in). `dest` must not already
    /// exist. Only meaningful for an on-disk database: `VACUUM INTO` is a
    /// silent no-op from a `:memory:` source.
    pub async fn snapshot_to(&self, dest: &Path) -> Result<()> {
        // `dest` comes from `AppPaths`, never user input; `''` doubling and the
        // forward-slash form are belt-and-braces for the SQL string literal.
        let target = dest
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "''");
        let sql = format!("VACUUM INTO '{target}'");
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&self.pool)
            .await?;
        if !dest.is_file() {
            return Err(CoreError::Db(format!(
                "VACUUM INTO produced no file at {} (in-memory source?)",
                dest.display()
            )));
        }
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// Current UTC time as an RFC 3339 string, the on-disk timestamp format.
pub(crate) fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_database_has_the_v1_schema() {
        let db = Database::connect_in_memory().await.unwrap();
        let tables = db.table_names().await.unwrap();
        for expected in [
            "settings",
            "runtimes",
            "models",
            "model_roles",
            "model_links",
            "jobs",
            "job_events",
            "voice_identities",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "missing table {expected}"
            );
        }
    }

    #[tokio::test]
    async fn snapshot_to_writes_a_readable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::connect(&dir.path().join("live.db"))
            .await
            .unwrap();
        db.settings().set("marker", "kept").await.unwrap();

        let snap = dir.path().join("snap.db");
        db.snapshot_to(&snap).await.unwrap();

        let reopened = Database::connect(&snap).await.unwrap();
        assert_eq!(
            reopened.settings().get("marker").await.unwrap().as_deref(),
            Some("kept")
        );
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let db = Database::connect_in_memory().await.unwrap();
        // Running the migrator again must be a no-op, not an error.
        MIGRATOR.run(&db.pool).await.unwrap();
    }

    #[tokio::test]
    async fn connect_creates_the_file_and_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("aiwm.db");

        {
            let db = Database::connect(&file).await.unwrap();
            db.settings().set("marker", "kept").await.unwrap();
            db.close().await;
        }
        assert!(file.is_file());

        let reopened = Database::connect(&file).await.unwrap();
        assert_eq!(
            reopened.settings().get("marker").await.unwrap().as_deref(),
            Some("kept")
        );
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let db = Database::connect_in_memory().await.unwrap();
        // model_roles references models(id); an orphan role must be rejected.
        let res = sqlx::query("INSERT INTO model_roles (model_id, role) VALUES ('nope', 'coding')")
            .execute(&db.pool)
            .await;
        assert!(res.is_err(), "foreign key violation should be rejected");
    }
}
