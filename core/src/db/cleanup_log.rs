//! The cleanup history (Plan 13): one row per entry a real cleanup apply
//! worked on. Written by [`crate::cleanup::apply`], read by the Settings
//! "Cleanup" page's history list. No foreign keys — the log outlives the
//! jobs, datasets and runs it is about.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// One line of the cleanup history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanupLogEntry {
    pub id: String,
    /// RFC 3339, UTC.
    pub ts: String,
    /// One of the scan's group keys.
    pub group_key: String,
    /// The scan's entry id (a file name, a dataset or run id, a folder name).
    pub entry_id: String,
    /// What the page showed for the entry.
    pub entry_label: String,
    pub deleted_files: u64,
    pub freed_bytes: u64,
    /// Frame rows removed, else 0.
    pub removed_rows: u64,
    pub skipped_count: u64,
    /// Skip reasons and a capped path list, as stored (`{}` when nothing).
    pub detail: serde_json::Value,
}

/// What an apply records for one entry. `id` and `ts` are set here.
#[derive(Debug, Clone)]
pub struct NewCleanupLogEntry {
    pub group_key: String,
    pub entry_id: String,
    pub entry_label: String,
    pub deleted_files: u64,
    pub freed_bytes: u64,
    pub removed_rows: u64,
    pub skipped_count: u64,
    pub detail: serde_json::Value,
}

#[derive(Debug)]
pub struct CleanupLogRepo<'a> {
    pool: &'a SqlitePool,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    ts: String,
    group_key: String,
    entry_id: String,
    entry_label: String,
    deleted_files: i64,
    freed_bytes: i64,
    removed_rows: i64,
    skipped_count: i64,
    detail_json: String,
}

impl From<Row> for CleanupLogEntry {
    fn from(r: Row) -> Self {
        let detail = serde_json::from_str(&r.detail_json).unwrap_or_else(|err| {
            tracing::warn!(
                entry = %r.id,
                %err,
                "stored cleanup detail is not valid JSON — showing it empty"
            );
            serde_json::json!({})
        });
        let unsigned = |v: i64| u64::try_from(v).unwrap_or(0);
        Self {
            id: r.id,
            ts: r.ts,
            group_key: r.group_key,
            entry_id: r.entry_id,
            entry_label: r.entry_label,
            deleted_files: unsigned(r.deleted_files),
            freed_bytes: unsigned(r.freed_bytes),
            removed_rows: unsigned(r.removed_rows),
            skipped_count: unsigned(r.skipped_count),
            detail,
        }
    }
}

const COLS: &str = "id, ts, group_key, entry_id, entry_label, deleted_files, freed_bytes, \
     removed_rows, skipped_count, detail_json";

/// Most rows one [`CleanupLogRepo::list`] returns; the page asks for 20.
pub const MAX_LIST_LIMIT: i64 = 200;

impl<'a> CleanupLogRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Record one applied entry.
    pub async fn insert(&self, new: NewCleanupLogEntry) -> Result<CleanupLogEntry> {
        let id = Uuid::now_v7().to_string();
        let ts = now_rfc3339();
        let signed = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        sqlx::query(
            "INSERT INTO cleanup_log (id, ts, group_key, entry_id, entry_label, deleted_files,
                 freed_bytes, removed_rows, skipped_count, detail_json)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(&id)
        .bind(&ts)
        .bind(&new.group_key)
        .bind(&new.entry_id)
        .bind(&new.entry_label)
        .bind(signed(new.deleted_files))
        .bind(signed(new.freed_bytes))
        .bind(signed(new.removed_rows))
        .bind(signed(new.skipped_count))
        .bind(new.detail.to_string())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("cleanup log row vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<CleanupLogEntry>> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM cleanup_log WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(CleanupLogEntry::from))
    }

    /// The newest `limit` rows (clamped to `1..=200`), newest first.
    pub async fn list(&self, limit: i64) -> Result<Vec<CleanupLogEntry>> {
        let limit = limit.clamp(1, MAX_LIST_LIMIT);
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM cleanup_log ORDER BY ts DESC, id DESC LIMIT $1"
        )))
        .bind(limit)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(CleanupLogEntry::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    use super::*;

    fn row(group: &str, entry: &str) -> NewCleanupLogEntry {
        NewCleanupLogEntry {
            group_key: group.into(),
            entry_id: entry.into(),
            entry_label: format!("label of {entry}"),
            deleted_files: 3,
            freed_bytes: 4_096,
            removed_rows: 0,
            skipped_count: 1,
            detail: serde_json::json!({ "skipped": [{ "path": "x", "reason": "missing" }] }),
        }
    }

    #[tokio::test]
    async fn insert_returns_the_stored_row_with_a_timestamp() {
        let db = Database::connect_in_memory().await.unwrap();

        let e = db
            .cleanup_log()
            .insert(row("media_orphans", "stray.png"))
            .await
            .unwrap();

        assert!(!e.id.is_empty());
        assert!(e.ts.contains('T') && e.ts.ends_with('Z'), "{}", e.ts);
        assert_eq!(e.group_key, "media_orphans");
        assert_eq!(e.entry_id, "stray.png");
        assert_eq!(e.entry_label, "label of stray.png");
        assert_eq!(e.deleted_files, 3);
        assert_eq!(e.freed_bytes, 4_096);
        assert_eq!(e.removed_rows, 0);
        assert_eq!(e.skipped_count, 1);
        assert_eq!(e.detail["skipped"][0]["reason"], "missing");
    }

    #[tokio::test]
    async fn list_is_newest_first_and_honours_the_limit() {
        let db = Database::connect_in_memory().await.unwrap();
        for i in 0..5 {
            db.cleanup_log()
                .insert(row("db_backups", &format!("b{i}.zip")))
                .await
                .unwrap();
        }

        let all = db.cleanup_log().list(20).await.unwrap();
        assert_eq!(all.len(), 5);
        let ids: Vec<&str> = all.iter().map(|e| e.entry_id.as_str()).collect();
        assert_eq!(ids, ["b4.zip", "b3.zip", "b2.zip", "b1.zip", "b0.zip"]);

        let two = db.cleanup_log().list(2).await.unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].entry_id, "b4.zip");
        // A bad limit is clamped, never an error.
        assert_eq!(db.cleanup_log().list(0).await.unwrap().len(), 1);
        assert_eq!(db.cleanup_log().list(-7).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_table_has_no_foreign_keys_so_rows_outlive_their_subjects() {
        let db = Database::connect_in_memory().await.unwrap();
        // A dataset id that never existed is fine: nothing references it.
        db.cleanup_log()
            .insert(row("discarded_frames", "no-such-dataset"))
            .await
            .unwrap();
        assert_eq!(db.cleanup_log().list(20).await.unwrap().len(), 1);
    }
}
