//! The `runtimes` table: which managed runtimes are installed, at what version,
//! and their last known state. The filesystem stays the source of truth for
//! "is the binary there" (see `runtime::llamacpp`); this table adds version,
//! install time and the last error for the Diagnostics view.

use serde::Serialize;
use sqlx::SqlitePool;

use super::now_rfc3339;
use crate::Result;

/// Lifecycle states stored in `runtimes.state`.
pub mod state {
    pub const NOT_INSTALLED: &str = "not_installed";
    pub const INSTALLING: &str = "installing";
    pub const STOPPED: &str = "stopped";
    pub const RUNNING: &str = "running";
    pub const ERROR: &str = "error";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct RuntimeRecord {
    pub id: String,
    pub kind: String,
    pub version: Option<String>,
    pub install_path: Option<String>,
    pub state: String,
    pub last_health: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug)]
pub struct RuntimeRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> RuntimeRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, id: &str) -> Result<Option<RuntimeRecord>> {
        let row = sqlx::query_as::<_, RuntimeRecord>(
            "SELECT id, kind, version, install_path, state, last_health, last_error
             FROM runtimes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    pub async fn all(&self) -> Result<Vec<RuntimeRecord>> {
        let rows = sqlx::query_as::<_, RuntimeRecord>(
            "SELECT id, kind, version, install_path, state, last_health, last_error
             FROM runtimes ORDER BY id",
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    /// Insert or update the whole row (keyed by `id`).
    pub async fn upsert(&self, r: &RuntimeRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO runtimes (id, kind, version, install_path, state, last_health, last_error)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 version = excluded.version,
                 install_path = excluded.install_path,
                 state = excluded.state,
                 last_health = excluded.last_health,
                 last_error = excluded.last_error",
        )
        .bind(&r.id)
        .bind(&r.kind)
        .bind(&r.version)
        .bind(&r.install_path)
        .bind(&r.state)
        .bind(&r.last_health)
        .bind(&r.last_error)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Set just the state (+ optional error), leaving version / path intact.
    /// Creates the row with `kind` if it does not exist yet.
    pub async fn set_state(
        &self,
        id: &str,
        kind: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO runtimes (id, kind, state, last_error)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(id) DO UPDATE SET state = excluded.state, last_error = excluded.last_error",
        )
        .bind(id)
        .bind(kind)
        .bind(state)
        .bind(error)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Record a successful install: version + path, state → `stopped`, error cleared.
    pub async fn record_install(
        &self,
        id: &str,
        kind: &str,
        version: &str,
        install_path: &str,
    ) -> Result<()> {
        self.upsert(&RuntimeRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            version: Some(version.to_string()),
            install_path: Some(install_path.to_string()),
            state: state::STOPPED.to_string(),
            last_health: None,
            last_error: None,
        })
        .await
    }

    /// Stamp `last_health` with the current time.
    pub async fn mark_health(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE runtimes SET last_health = $1 WHERE id = $2")
            .bind(now_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn absent_runtime_is_none() {
        let db = Database::connect_in_memory().await.unwrap();
        assert!(db.runtimes().get("llamacpp").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_state_creates_then_updates() {
        let db = Database::connect_in_memory().await.unwrap();
        let r = db.runtimes();

        r.set_state("llamacpp", "llama_cpp", state::INSTALLING, None)
            .await
            .unwrap();
        assert_eq!(
            r.get("llamacpp").await.unwrap().unwrap().state,
            state::INSTALLING
        );

        r.set_state("llamacpp", "llama_cpp", state::ERROR, Some("boom"))
            .await
            .unwrap();
        let got = r.get("llamacpp").await.unwrap().unwrap();
        assert_eq!(got.state, state::ERROR);
        assert_eq!(got.last_error.as_deref(), Some("boom"));
        assert!(got.version.is_none(), "state change must not touch version");
    }

    #[tokio::test]
    async fn record_install_sets_version_path_and_clears_error() {
        let db = Database::connect_in_memory().await.unwrap();
        let r = db.runtimes();
        r.set_state(
            "llamacpp",
            "llama_cpp",
            state::ERROR,
            Some("earlier failure"),
        )
        .await
        .unwrap();

        r.record_install(
            "llamacpp",
            "llama_cpp",
            "b10855",
            "C:\\rt\\llamacpp\\b10855",
        )
        .await
        .unwrap();

        let got = r.get("llamacpp").await.unwrap().unwrap();
        assert_eq!(got.version.as_deref(), Some("b10855"));
        assert_eq!(
            got.install_path.as_deref(),
            Some("C:\\rt\\llamacpp\\b10855")
        );
        assert_eq!(got.state, state::STOPPED);
        assert!(got.last_error.is_none());
    }

    #[tokio::test]
    async fn all_lists_every_runtime_sorted() {
        let db = Database::connect_in_memory().await.unwrap();
        let r = db.runtimes();
        r.set_state("comfyui", "comfy_ui", state::NOT_INSTALLED, None)
            .await
            .unwrap();
        r.set_state("llamacpp", "llama_cpp", state::STOPPED, None)
            .await
            .unwrap();

        let ids: Vec<_> = r.all().await.unwrap().into_iter().map(|x| x.id).collect();
        assert_eq!(ids, ["comfyui", "llamacpp"]);
    }

    #[tokio::test]
    async fn mark_health_stamps_a_timestamp() {
        let db = Database::connect_in_memory().await.unwrap();
        let r = db.runtimes();
        r.record_install("llamacpp", "llama_cpp", "b1", "p")
            .await
            .unwrap();
        assert!(r
            .get("llamacpp")
            .await
            .unwrap()
            .unwrap()
            .last_health
            .is_none());

        r.mark_health("llamacpp").await.unwrap();
        assert!(r
            .get("llamacpp")
            .await
            .unwrap()
            .unwrap()
            .last_health
            .is_some());
    }
}
