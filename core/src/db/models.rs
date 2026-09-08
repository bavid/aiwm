//! Model registry persistence: the `models` table plus `model_roles`.
//! `model_links` (per-runtime junctions) arrives with the link manager.

use std::collections::BTreeMap;

use serde::Serialize;
use sqlx::SqlitePool;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// A model as stored, with its roles attached.
#[derive(Debug, Clone, Serialize)]
pub struct Model {
    pub id: String,
    pub publisher: Option<String>,
    pub name: String,
    pub family: Option<String>,
    pub format: String,
    pub quant: Option<String>,
    pub arch: Option<String>,
    pub param_count: Option<i64>,
    pub file_path: String,
    pub sha256: Option<String>,
    pub size_bytes: i64,
    pub ctx_max: Option<i64>,
    pub vram_estimate_mb: Option<i64>,
    pub ram_estimate_mb: Option<i64>,
    pub source: String,
    pub source_revision: Option<String>,
    pub imported_at: String,
    pub last_used_at: Option<String>,
    pub use_count: i64,
    pub roles: Vec<String>,
}

/// Fields supplied when registering a model. `id` and `imported_at` are set here.
#[derive(Debug, Clone, Default)]
pub struct NewModel {
    pub publisher: Option<String>,
    pub name: String,
    pub family: Option<String>,
    pub format: String,
    pub quant: Option<String>,
    pub arch: Option<String>,
    pub param_count: Option<i64>,
    pub file_path: String,
    pub sha256: Option<String>,
    pub size_bytes: i64,
    pub ctx_max: Option<i64>,
    pub vram_estimate_mb: Option<i64>,
    pub ram_estimate_mb: Option<i64>,
    pub source: String,
    pub source_revision: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug)]
pub struct ModelRepo<'a> {
    pool: &'a SqlitePool,
}

#[derive(sqlx::FromRow)]
struct ModelRow {
    id: String,
    publisher: Option<String>,
    name: String,
    family: Option<String>,
    format: String,
    quant: Option<String>,
    arch: Option<String>,
    param_count: Option<i64>,
    file_path: String,
    sha256: Option<String>,
    size_bytes: i64,
    ctx_max: Option<i64>,
    vram_estimate_mb: Option<i64>,
    ram_estimate_mb: Option<i64>,
    source: String,
    source_revision: Option<String>,
    imported_at: String,
    last_used_at: Option<String>,
    use_count: i64,
}

impl ModelRow {
    fn into_model(self, roles: Vec<String>) -> Model {
        Model {
            id: self.id,
            publisher: self.publisher,
            name: self.name,
            family: self.family,
            format: self.format,
            quant: self.quant,
            arch: self.arch,
            param_count: self.param_count,
            file_path: self.file_path,
            sha256: self.sha256,
            size_bytes: self.size_bytes,
            ctx_max: self.ctx_max,
            vram_estimate_mb: self.vram_estimate_mb,
            ram_estimate_mb: self.ram_estimate_mb,
            source: self.source,
            source_revision: self.source_revision,
            imported_at: self.imported_at,
            last_used_at: self.last_used_at,
            use_count: self.use_count,
            roles,
        }
    }
}

const COLS: &str = "id, publisher, name, family, format, quant, arch, param_count, file_path, \
     sha256, size_bytes, ctx_max, vram_estimate_mb, ram_estimate_mb, source, source_revision, \
     imported_at, last_used_at, use_count";

impl<'a> ModelRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewModel) -> Result<Model> {
        if new.name.trim().is_empty() {
            return Err(CoreError::Db("model name must not be empty".into()));
        }
        let id = uuid::Uuid::now_v7().to_string();
        let now = now_rfc3339();

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO models (id, publisher, name, family, format, quant, arch, param_count,
                 file_path, sha256, size_bytes, ctx_max, vram_estimate_mb, ram_estimate_mb,
                 source, source_revision, imported_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
        )
        .bind(&id)
        .bind(&new.publisher)
        .bind(&new.name)
        .bind(&new.family)
        .bind(&new.format)
        .bind(&new.quant)
        .bind(&new.arch)
        .bind(new.param_count)
        .bind(&new.file_path)
        .bind(&new.sha256)
        .bind(new.size_bytes)
        .bind(new.ctx_max)
        .bind(new.vram_estimate_mb)
        .bind(new.ram_estimate_mb)
        .bind(&new.source)
        .bind(&new.source_revision)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        for role in dedup_sorted(&new.roles) {
            sqlx::query("INSERT OR IGNORE INTO model_roles (model_id, role) VALUES ($1, $2)")
                .bind(&id)
                .bind(&role)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("model vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Model>> {
        let row: Option<ModelRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM models WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        match row {
            Some(r) => Ok(Some(r.into_model(self.roles(id).await?))),
            None => Ok(None),
        }
    }

    pub async fn list(&self) -> Result<Vec<Model>> {
        let rows: Vec<ModelRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM models ORDER BY name COLLATE NOCASE"
        )))
        .fetch_all(self.pool)
        .await?;

        let roles = self.all_roles().await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let rs = roles.get(&r.id).cloned().unwrap_or_default();
                r.into_model(rs)
            })
            .collect())
    }

    pub async fn find_by_sha256(&self, sha256: &str) -> Result<Option<Model>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM models WHERE sha256 = $1 LIMIT 1")
                .bind(sha256)
                .fetch_optional(self.pool)
                .await?;
        match row {
            Some((id,)) => self.get(&id).await,
            None => Ok(None),
        }
    }

    pub async fn find_by_path(&self, path: &str) -> Result<Option<Model>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM models WHERE file_path = $1 LIMIT 1")
                .bind(path)
                .fetch_optional(self.pool)
                .await?;
        match row {
            Some((id,)) => self.get(&id).await,
            None => Ok(None),
        }
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM models WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn mark_used(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE models SET use_count = use_count + 1, last_used_at = $1 WHERE id = $2")
            .bind(now_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn roles(&self, id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT role FROM model_roles WHERE model_id = $1 ORDER BY role")
                .bind(id)
                .fetch_all(self.pool)
                .await?;
        Ok(rows.into_iter().map(|(r,)| r).collect())
    }

    async fn all_roles(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT model_id, role FROM model_roles ORDER BY role")
                .fetch_all(self.pool)
                .await?;
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, role) in rows {
            map.entry(id).or_default().push(role);
        }
        Ok(map)
    }
}

fn dedup_sorted(roles: &[String]) -> Vec<String> {
    let mut v: Vec<String> = roles
        .iter()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, NewModel};

    fn gguf_model(name: &str, sha: &str) -> NewModel {
        NewModel {
            name: name.into(),
            format: "gguf".into(),
            quant: Some("Q4_K_M".into()),
            arch: Some("qwen2".into()),
            file_path: format!("E:\\AI\\models\\llm\\{name}\\model.gguf"),
            sha256: Some(sha.into()),
            size_bytes: 9_000_000_000,
            ctx_max: Some(32768),
            vram_estimate_mb: Some(9_600),
            source: "manual".into(),
            roles: vec!["coding".into(), "chat".into(), "coding".into()],
            ..NewModel::default()
        }
    }

    #[tokio::test]
    async fn insert_get_list_with_roles() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db
            .models()
            .insert(gguf_model("qwen-14b", "abc"))
            .await
            .unwrap();

        assert_eq!(m.name, "qwen-14b");
        assert_eq!(m.roles, ["chat", "coding"]); // deduped + sorted
        assert_eq!(m.format, "gguf");

        let got = db.models().get(&m.id).await.unwrap().unwrap();
        assert_eq!(got.id, m.id);
        assert_eq!(got.roles, m.roles);

        let all = db.models().list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].roles, ["chat", "coding"]);
    }

    #[tokio::test]
    async fn find_by_sha256_and_path() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db
            .models()
            .insert(gguf_model("m", "deadbeef"))
            .await
            .unwrap();

        assert_eq!(
            db.models()
                .find_by_sha256("deadbeef")
                .await
                .unwrap()
                .unwrap()
                .id,
            m.id
        );
        assert!(db.models().find_by_sha256("nope").await.unwrap().is_none());
        assert_eq!(
            db.models()
                .find_by_path(&m.file_path)
                .await
                .unwrap()
                .unwrap()
                .id,
            m.id
        );
    }

    #[tokio::test]
    async fn unique_file_path_is_enforced() {
        let db = Database::connect_in_memory().await.unwrap();
        db.models().insert(gguf_model("a", "h1")).await.unwrap();

        let mut dup = gguf_model("b", "h2");
        dup.file_path = "E:\\AI\\models\\llm\\a\\model.gguf".into();
        assert!(db.models().insert(dup).await.is_err());
    }

    #[tokio::test]
    async fn delete_cascades_roles() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db.models().insert(gguf_model("m", "h")).await.unwrap();

        assert!(db.models().delete(&m.id).await.unwrap());
        assert!(db.models().get(&m.id).await.unwrap().is_none());
        assert!(db.models().roles(&m.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mark_used_bumps_count_and_timestamp() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db.models().insert(gguf_model("m", "h")).await.unwrap();
        assert_eq!(m.use_count, 0);

        db.models().mark_used(&m.id).await.unwrap();
        let got = db.models().get(&m.id).await.unwrap().unwrap();
        assert_eq!(got.use_count, 1);
        assert!(got.last_used_at.is_some());
    }

    #[tokio::test]
    async fn empty_name_is_rejected() {
        let db = Database::connect_in_memory().await.unwrap();
        let mut bad = gguf_model("x", "h");
        bad.name = "  ".into();
        assert!(db.models().insert(bad).await.is_err());
    }
}
