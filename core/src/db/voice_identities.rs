//! Named Dia voice-cloning identities: a reference clip + its transcript,
//! saved once under a name (e.g. "Old Man Gareth") and reused across many
//! narration calls instead of re-picking a file and re-typing the
//! transcript every time. This repo is pure storage — the file-copy step
//! that runs before a row lands here lives in [`crate::voice_identity`]
//! (needs [`crate::paths::AppPaths`], not just the DB); see
//! `capability::tts` for how a Dia request resolves a saved identity into
//! sidecar params.

use serde::Serialize;
use sqlx::{AssertSqlSafe, SqlitePool};
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// A stored voice identity.
#[derive(Debug, Clone, Serialize)]
pub struct VoiceIdentity {
    pub id: String,
    pub name: String,
    /// Always a path under `AppPaths::voice_identities_dir` — never the
    /// user's original file location (see the module docs).
    pub reference_audio_path: String,
    pub reference_transcript: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct VoiceIdentityRow {
    id: String,
    name: String,
    reference_audio_path: String,
    reference_transcript: String,
    created_at: String,
}

impl From<VoiceIdentityRow> for VoiceIdentity {
    fn from(r: VoiceIdentityRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            reference_audio_path: r.reference_audio_path,
            reference_transcript: r.reference_transcript,
            created_at: r.created_at,
        }
    }
}

// The `SELECT` statements below interpolate only this compile-time constant
// and `$N` bind placeholders — never caller data (always bound). `AssertSqlSafe`
// documents that we have checked this.
const SELECT_COLS: &str = "id, name, reference_audio_path, reference_transcript, created_at";

#[derive(Debug)]
pub struct VoiceIdentityRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> VoiceIdentityRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// `reference_audio_path` must already be a file AIWM itself owns (a
    /// copy under `voice_identities_dir`, not the caller's original pick) —
    /// this repo has no opinion on that, it just stores the path it's given.
    pub async fn create(
        &self,
        name: &str,
        reference_audio_path: &str,
        reference_transcript: &str,
    ) -> Result<VoiceIdentity> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO voice_identities \
             (id, name, reference_audio_path, reference_transcript, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(name)
        .bind(reference_audio_path)
        .bind(reference_transcript)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("voice identity vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<VoiceIdentity>> {
        let row = sqlx::query_as::<_, VoiceIdentityRow>(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM voice_identities WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(VoiceIdentity::from))
    }

    /// Every saved identity, most recently created first.
    pub async fn list(&self) -> Result<Vec<VoiceIdentity>> {
        let rows = sqlx::query_as::<_, VoiceIdentityRow>(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM voice_identities ORDER BY created_at DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(VoiceIdentity::from).collect())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM voice_identities WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    async fn db() -> Database {
        Database::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let db = db().await;
        let v = db
            .voice_identities()
            .create(
                "Old Man Gareth",
                "E:\\AI\\data\\voice-identities\\abc\\ref.wav",
                "hello there",
            )
            .await
            .unwrap();
        assert_eq!(v.name, "Old Man Gareth");
        assert_eq!(v.reference_transcript, "hello there");

        let fetched = db.voice_identities().get(&v.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, v.id);
        assert_eq!(fetched.reference_audio_path, v.reference_audio_path);
    }

    #[tokio::test]
    async fn get_returns_none_for_an_unknown_id() {
        let db = db().await;
        assert!(db.voice_identities().get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_returns_every_identity_most_recent_first() {
        let db = db().await;
        let a = db
            .voice_identities()
            .create("A", "path/a.wav", "transcript a")
            .await
            .unwrap();
        let b = db
            .voice_identities()
            .create("B", "path/b.wav", "transcript b")
            .await
            .unwrap();

        let all = db.voice_identities().list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, b.id, "most recently created first");
        assert_eq!(all[1].id, a.id);
    }

    #[tokio::test]
    async fn delete_removes_the_identity() {
        let db = db().await;
        let v = db
            .voice_identities()
            .create("Gone Soon", "path/x.wav", "x")
            .await
            .unwrap();

        db.voice_identities().delete(&v.id).await.unwrap();

        assert!(db.voice_identities().get(&v.id).await.unwrap().is_none());
        assert!(db.voice_identities().list().await.unwrap().is_empty());
    }
}
