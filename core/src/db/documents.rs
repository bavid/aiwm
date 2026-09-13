//! Documents attached to a chat session for local RAG (7.x). Session-scoped
//! and deleted along with it (`ON DELETE CASCADE` — see the migration's own
//! comment for why this differs from `sessions.delete`'s orphan-not-delete
//! behavior for jobs).

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// A document attached to a chat session.
#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub id: String,
    pub session_id: String,
    /// Original file name, for display.
    pub name: String,
    pub source_path: String,
    /// `"txt"` | `"md"`.
    pub format: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    session_id: String,
    name: String,
    source_path: String,
    format: String,
    created_at: String,
}

impl From<DocumentRow> for Document {
    fn from(r: DocumentRow) -> Self {
        Self {
            id: r.id,
            session_id: r.session_id,
            name: r.name,
            source_path: r.source_path,
            format: r.format,
            created_at: r.created_at,
        }
    }
}

/// One chunk of a document's text — what retrieval actually scores against.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DocumentChunk {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i64,
    pub text: String,
}

/// Body for [`DocumentRepo::insert`].
#[derive(Debug)]
pub struct NewDocument {
    pub session_id: String,
    pub name: String,
    pub source_path: String,
    pub format: String,
}

const SELECT_DOC_COLS: &str = "id, session_id, name, source_path, format, created_at";

#[derive(Debug)]
pub struct DocumentRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DocumentRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert the document row plus every chunk (already split by the caller
    /// — chunking is a pure function in `crate::rag`, not this repo's job).
    pub async fn insert(&self, doc: NewDocument, chunks: &[String]) -> Result<Document> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO documents (id, session_id, name, source_path, format, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(&doc.session_id)
        .bind(&doc.name)
        .bind(&doc.source_path)
        .bind(&doc.format)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;

        for (index, text) in chunks.iter().enumerate() {
            sqlx::query(
                "INSERT INTO document_chunks (id, document_id, chunk_index, text) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(&id)
            .bind(i64::try_from(index).unwrap_or(i64::MAX))
            .bind(text)
            .execute(self.pool)
            .await?;
        }

        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("document vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Document>> {
        let row = sqlx::query_as::<_, DocumentRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_DOC_COLS} FROM documents WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Document::from))
    }

    /// Every document attached to `session_id`, oldest first.
    pub async fn list_for_session(&self, session_id: &str) -> Result<Vec<Document>> {
        let rows = sqlx::query_as::<_, DocumentRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_DOC_COLS} FROM documents WHERE session_id = $1 ORDER BY created_at"
        )))
        .bind(session_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Document::from).collect())
    }

    /// Removes the document and every one of its chunks (`ON DELETE CASCADE`).
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM documents WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Every chunk across every document attached to `session_id` — the
    /// candidate set a chat turn's retrieval scores against.
    pub async fn chunks_for_session(&self, session_id: &str) -> Result<Vec<DocumentChunk>> {
        let rows = sqlx::query_as::<_, DocumentChunk>(
            "SELECT c.id, c.document_id, c.chunk_index, c.text \
             FROM document_chunks c \
             JOIN documents d ON d.id = c.document_id \
             WHERE d.session_id = $1 \
             ORDER BY d.created_at, c.chunk_index",
        )
        .bind(session_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    use super::NewDocument;

    async fn db_with_session() -> (Database, String) {
        let db = Database::connect_in_memory().await.unwrap();
        let s = db.sessions().create("chat", "Research").await.unwrap();
        (db, s.id)
    }

    fn new_doc(session_id: &str) -> NewDocument {
        NewDocument {
            session_id: session_id.to_string(),
            name: "notes.md".into(),
            source_path: "C:\\docs\\notes.md".into(),
            format: "md".into(),
        }
    }

    #[tokio::test]
    async fn insert_stores_the_document_and_every_chunk() {
        let (db, session_id) = db_with_session().await;
        let chunks = vec!["first chunk".to_string(), "second chunk".to_string()];

        let doc = db
            .documents()
            .insert(new_doc(&session_id), &chunks)
            .await
            .unwrap();
        assert_eq!(doc.name, "notes.md");
        assert_eq!(doc.session_id, session_id);

        let stored = db
            .documents()
            .chunks_for_session(&session_id)
            .await
            .unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].text, "first chunk");
        assert_eq!(stored[0].chunk_index, 0);
        assert_eq!(stored[1].chunk_index, 1);
    }

    #[tokio::test]
    async fn list_for_session_only_returns_that_sessions_documents() {
        let (db, session_a) = db_with_session().await;
        let session_b = db.sessions().create("chat", "Other").await.unwrap().id;

        db.documents()
            .insert(new_doc(&session_a), &[])
            .await
            .unwrap();
        db.documents()
            .insert(new_doc(&session_b), &[])
            .await
            .unwrap();

        let docs = db.documents().list_for_session(&session_a).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].session_id, session_a);
    }

    #[tokio::test]
    async fn delete_removes_the_document_and_its_chunks() {
        let (db, session_id) = db_with_session().await;
        let doc = db
            .documents()
            .insert(new_doc(&session_id), &["a chunk".to_string()])
            .await
            .unwrap();

        db.documents().delete(&doc.id).await.unwrap();

        assert!(db.documents().get(&doc.id).await.unwrap().is_none());
        assert!(db
            .documents()
            .chunks_for_session(&session_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn deleting_the_session_cascades_to_its_documents() {
        let (db, session_id) = db_with_session().await;
        let doc = db
            .documents()
            .insert(new_doc(&session_id), &["a chunk".to_string()])
            .await
            .unwrap();

        db.sessions().delete(&session_id).await.unwrap();

        assert!(db.documents().get(&doc.id).await.unwrap().is_none());
    }
}
