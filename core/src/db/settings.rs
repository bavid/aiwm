//! Key/value application settings. App-managed state (schema version, install
//! markers, remembered selections) — *not* the user's `config.toml`, which stays
//! the single source of truth for startup configuration.

use std::collections::BTreeMap;

use sqlx::SqlitePool;

use super::now_rfc3339;
use crate::Result;

#[derive(Debug)]
pub struct SettingsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SettingsRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(self.pool)
            .await?;
        Ok(row.map(|(v,)| v))
    }

    /// Insert or overwrite.
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Insert only if the key is not already present. Returns whether a row was
    /// written. Used for first-run seeding.
    pub async fn set_if_absent(&self, key: &str, value: &str) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT(key) DO NOTHING",
        )
        .bind(key)
        .bind(value)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn all(&self) -> Result<BTreeMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM settings ORDER BY key")
                .fetch_all(self.pool)
                .await?;
        Ok(rows.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    #[tokio::test]
    async fn set_then_get_roundtrips() {
        let db = Database::connect_in_memory().await.unwrap();
        assert_eq!(db.settings().get("missing").await.unwrap(), None);

        db.settings().set("theme", "dark").await.unwrap();
        assert_eq!(
            db.settings().get("theme").await.unwrap().as_deref(),
            Some("dark")
        );
    }

    #[tokio::test]
    async fn set_overwrites_existing_value() {
        let db = Database::connect_in_memory().await.unwrap();
        db.settings().set("k", "one").await.unwrap();
        db.settings().set("k", "two").await.unwrap();
        assert_eq!(
            db.settings().get("k").await.unwrap().as_deref(),
            Some("two")
        );
    }

    #[tokio::test]
    async fn set_if_absent_only_writes_once() {
        let db = Database::connect_in_memory().await.unwrap();

        assert!(db
            .settings()
            .set_if_absent("schema_version", "1")
            .await
            .unwrap());
        assert!(!db
            .settings()
            .set_if_absent("schema_version", "999")
            .await
            .unwrap());

        assert_eq!(
            db.settings()
                .get("schema_version")
                .await
                .unwrap()
                .as_deref(),
            Some("1")
        );
    }

    #[tokio::test]
    async fn all_returns_sorted_pairs() {
        let db = Database::connect_in_memory().await.unwrap();
        db.settings().set("b", "2").await.unwrap();
        db.settings().set("a", "1").await.unwrap();

        let all = db.settings().all().await.unwrap();
        let keys: Vec<_> = all.keys().cloned().collect();
        assert_eq!(keys, ["a", "b"]);
        assert_eq!(all["a"], "1");
    }
}
