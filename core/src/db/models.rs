//! Model registry persistence: the `models` table, `model_roles`, and
//! `model_links` (which runtimes can reach the file, and how — ADR-007).

use std::collections::BTreeMap;

use serde::Serialize;
use sqlx::SqlitePool;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// How one runtime reaches a model's canonical file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct ModelLink {
    pub runtime_id: String,
    pub strategy: String,
    pub link_path: String,
}

/// A model as stored, with its roles + runtime links attached.
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
    /// Transformer layers (`block_count`) — for the VRAM / KV-cache estimate.
    pub n_layers: Option<i64>,
    /// Hidden size (`embedding_length`).
    pub n_embd: Option<i64>,
    /// Attention heads (`attention.head_count`).
    pub n_heads: Option<i64>,
    /// KV heads (`attention.head_count_kv`).
    pub n_kv_heads: Option<i64>,
    pub roles: Vec<String>,
    /// Runtime ids that can use this model (from `model_links`).
    pub runtimes: Vec<String>,
}

impl Model {
    /// Architecture facts for the VRAM fit estimate ([`crate::compat`]).
    pub fn vram_dims(&self) -> crate::compat::ModelDims {
        let as_u32 = |v: Option<i64>| v.and_then(|n| u32::try_from(n).ok());
        crate::compat::ModelDims {
            size_bytes: u64::try_from(self.size_bytes).unwrap_or(0),
            ctx_max: as_u32(self.ctx_max),
            param_count: self.param_count.and_then(|n| u64::try_from(n).ok()),
            n_layers: as_u32(self.n_layers),
            n_embd: as_u32(self.n_embd),
            n_heads: as_u32(self.n_heads),
            n_kv_heads: as_u32(self.n_kv_heads),
        }
    }
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
    pub n_layers: Option<i64>,
    pub n_embd: Option<i64>,
    pub n_heads: Option<i64>,
    pub n_kv_heads: Option<i64>,
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
    n_layers: Option<i64>,
    n_embd: Option<i64>,
    n_heads: Option<i64>,
    n_kv_heads: Option<i64>,
}

impl ModelRow {
    fn into_model(self, roles: Vec<String>, runtimes: Vec<String>) -> Model {
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
            n_layers: self.n_layers,
            n_embd: self.n_embd,
            n_heads: self.n_heads,
            n_kv_heads: self.n_kv_heads,
            roles,
            runtimes,
        }
    }
}

const COLS: &str = "id, publisher, name, family, format, quant, arch, param_count, file_path, \
     sha256, size_bytes, ctx_max, vram_estimate_mb, ram_estimate_mb, source, source_revision, \
     imported_at, last_used_at, use_count, n_layers, n_embd, n_heads, n_kv_heads";

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
                 source, source_revision, imported_at, n_layers, n_embd, n_heads, n_kv_heads)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)",
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
        .bind(new.n_layers)
        .bind(new.n_embd)
        .bind(new.n_heads)
        .bind(new.n_kv_heads)
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
            Some(r) => {
                let runtimes = self
                    .links(id)
                    .await?
                    .into_iter()
                    .map(|l| l.runtime_id)
                    .collect();
                Ok(Some(r.into_model(self.roles(id).await?, runtimes)))
            }
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
        let runtimes = self.all_link_runtimes().await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let rs = roles.get(&r.id).cloned().unwrap_or_default();
                let rt = runtimes.get(&r.id).cloned().unwrap_or_default();
                r.into_model(rs, rt)
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

    /// The best candidate model for `role` when the caller asked for `Auto`:
    /// most-recently-used first, then most-used, then by name. `None` when no
    /// model carries that role.
    pub async fn pick_for_role(&self, role: &str) -> Result<Option<Model>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT m.id FROM models m
             JOIN model_roles r ON r.model_id = m.id
             WHERE r.role = $1
             ORDER BY m.last_used_at DESC, m.use_count DESC, m.name COLLATE NOCASE
             LIMIT 1",
        )
        .bind(role)
        .fetch_optional(self.pool)
        .await?;
        match row {
            Some((id,)) => self.get(&id).await,
            None => Ok(None),
        }
    }

    /// Every model carrying `role`, name-sorted. Used when one role can have
    /// several members that must be told apart (Flux's T5 + CLIP-L encoders).
    pub async fn for_role(&self, role: &str) -> Result<Vec<Model>> {
        let ids: Vec<(String,)> = sqlx::query_as(
            "SELECT m.id FROM models m
             JOIN model_roles r ON r.model_id = m.id
             WHERE r.role = $1
             ORDER BY m.name COLLATE NOCASE",
        )
        .bind(role)
        .fetch_all(self.pool)
        .await?;
        let mut out = Vec::with_capacity(ids.len());
        for (id,) in ids {
            if let Some(m) = self.get(&id).await? {
                out.push(m);
            }
        }
        Ok(out)
    }

    /// Every model carrying `role`, each paired with its most recent benchmark
    /// (`None` if never tested). Feeds [`crate::select::pick_for_role`].
    pub async fn for_role_with_benchmark(
        &self,
        role: &str,
    ) -> Result<Vec<(Model, Option<super::Benchmark>)>> {
        let models = self.for_role(role).await?;
        let bench = super::BenchRepo::new(self.pool);
        let mut out = Vec::with_capacity(models.len());
        for m in models {
            let b = bench.latest_for(&m.id).await?;
            out.push((m, b));
        }
        Ok(out)
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

    /// Replace this model's role set (trimmed, de-duped, sorted — same
    /// cleaning as at import time). Lets a model fixed up after the fact —
    /// e.g. a chat GGUF downloaded before its `coding` role was set — get the
    /// role without re-importing. Doesn't check the role names against a
    /// fixed list: the callers (the Settings-style checkbox UI, `import_model`)
    /// already constrain that.
    pub async fn set_roles(&self, id: &str, roles: &[String]) -> Result<Vec<String>> {
        let clean = dedup_sorted(roles);

        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM model_roles WHERE model_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for role in &clean {
            sqlx::query("INSERT INTO model_roles (model_id, role) VALUES ($1, $2)")
                .bind(id)
                .bind(role)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(clean)
    }

    // --- tags (model_tags, 6.9) ---------------------------------------------

    /// This model's tags, alphabetical.
    pub async fn tags(&self, id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT tag FROM model_tags WHERE model_id = $1 ORDER BY tag")
                .bind(id)
                .fetch_all(self.pool)
                .await?;
        Ok(rows.into_iter().map(|(t,)| t).collect())
    }

    /// Replace this model's tag set. Tags are trimmed, lower-cased, de-duped;
    /// empties and anything over 32 chars are dropped.
    pub async fn set_tags(&self, id: &str, tags: &[String]) -> Result<Vec<String>> {
        let mut clean: Vec<String> = tags
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty() && t.chars().count() <= 32)
            .collect();
        clean.sort();
        clean.dedup();

        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM model_tags WHERE model_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for tag in &clean {
            sqlx::query("INSERT INTO model_tags (model_id, tag) VALUES ($1, $2)")
                .bind(id)
                .bind(tag)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(clean)
    }

    /// `model_id -> [tags]` for every tagged model.
    pub async fn all_tags(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT model_id, tag FROM model_tags ORDER BY tag")
                .fetch_all(self.pool)
                .await?;
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, tag) in rows {
            map.entry(id).or_default().push(tag);
        }
        Ok(map)
    }

    // --- links (model_links) -------------------------------------------------

    /// Record how `runtime_id` reaches this model's file (ADR-007). Upsert.
    pub async fn link_runtime(
        &self,
        model_id: &str,
        runtime_id: &str,
        strategy: &str,
        link_path: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO model_links (model_id, runtime_id, strategy, link_path)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(model_id, runtime_id)
             DO UPDATE SET strategy = excluded.strategy, link_path = excluded.link_path",
        )
        .bind(model_id)
        .bind(runtime_id)
        .bind(strategy)
        .bind(link_path)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn unlink_runtime(&self, model_id: &str, runtime_id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM model_links WHERE model_id = $1 AND runtime_id = $2")
            .bind(model_id)
            .bind(runtime_id)
            .execute(self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn links(&self, model_id: &str) -> Result<Vec<ModelLink>> {
        let rows = sqlx::query_as::<_, ModelLink>(
            "SELECT runtime_id, strategy, link_path FROM model_links
             WHERE model_id = $1 ORDER BY runtime_id",
        )
        .bind(model_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    async fn all_link_runtimes(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT model_id, runtime_id FROM model_links ORDER BY runtime_id")
                .fetch_all(self.pool)
                .await?;
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, runtime_id) in rows {
            map.entry(id).or_default().push(runtime_id);
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
            n_layers: Some(48),
            n_embd: Some(5120),
            n_heads: Some(40),
            n_kv_heads: Some(8),
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
    async fn arch_dims_round_trip_and_feed_the_estimate() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db.models().insert(gguf_model("q", "h")).await.unwrap();
        assert_eq!(m.n_layers, Some(48));
        assert_eq!(m.n_kv_heads, Some(8));

        let dims = m.vram_dims();
        assert_eq!(dims.n_layers, Some(48));
        assert_eq!(dims.n_embd, Some(5120));
        assert_eq!(dims.ctx_max, Some(32768));
        // The estimate has real dims to work with (not the rough fallback).
        assert!(!crate::compat::estimate(&dims, 8192).kv_is_rough);
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
    async fn delete_cascades_roles_and_links() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db.models().insert(gguf_model("m", "h")).await.unwrap();
        db.models()
            .link_runtime(&m.id, "llamacpp", "passthrough", &m.file_path)
            .await
            .unwrap();

        assert!(db.models().delete(&m.id).await.unwrap());
        assert!(db.models().get(&m.id).await.unwrap().is_none());
        assert!(db.models().roles(&m.id).await.unwrap().is_empty());
        assert!(db.models().links(&m.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn link_runtime_upserts_and_shows_on_the_model() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = db.models().insert(gguf_model("q", "h")).await.unwrap();
        assert!(m.runtimes.is_empty());

        db.models()
            .link_runtime(&m.id, "llamacpp", "passthrough", "E:\\c\\q.gguf")
            .await
            .unwrap();
        db.models()
            .link_runtime(&m.id, "llamacpp", "junction", "C:\\rt\\q") // upsert
            .await
            .unwrap();

        let got = db.models().get(&m.id).await.unwrap().unwrap();
        assert_eq!(got.runtimes, ["llamacpp"]);
        let links = db.models().links(&m.id).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].strategy, "junction");
        assert_eq!(links[0].link_path, "C:\\rt\\q");

        // Shows in `list` too.
        assert_eq!(db.models().list().await.unwrap()[0].runtimes, ["llamacpp"]);

        assert!(db.models().unlink_runtime(&m.id, "llamacpp").await.unwrap());
        assert!(db
            .models()
            .get(&m.id)
            .await
            .unwrap()
            .unwrap()
            .runtimes
            .is_empty());
    }

    #[tokio::test]
    async fn for_role_returns_all_members_name_sorted() {
        let db = Database::connect_in_memory().await.unwrap();
        assert!(db
            .models()
            .for_role("text_encoder")
            .await
            .unwrap()
            .is_empty());

        for (name, sha) in [("t5xxl_fp8", "h1"), ("clip_l", "h2")] {
            let mut m = gguf_model(name, sha);
            m.format = "safetensors".into();
            m.roles = vec!["text_encoder".into()];
            db.models().insert(m).await.unwrap();
        }
        let names: Vec<_> = db
            .models()
            .for_role("text_encoder")
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["clip_l", "t5xxl_fp8"]);
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

    #[tokio::test]
    async fn pick_for_role_prefers_most_recently_used() {
        let db = Database::connect_in_memory().await.unwrap();
        assert!(db.models().pick_for_role("chat").await.unwrap().is_none());

        let mut a = gguf_model("alpha", "ha");
        a.roles = vec!["chat".into()];
        let mut b = gguf_model("bravo", "hb");
        b.roles = vec!["chat".into()];
        let mut c = gguf_model("charlie", "hc");
        c.roles = vec!["coding".into()];
        let a = db.models().insert(a).await.unwrap();
        db.models().insert(b).await.unwrap();
        db.models().insert(c).await.unwrap();

        // No usage yet → alphabetical.
        assert_eq!(
            db.models()
                .pick_for_role("chat")
                .await
                .unwrap()
                .unwrap()
                .name,
            "alpha"
        );
        // Use bravo → it wins next time.
        let b_id = db.models().find_by_sha256("hb").await.unwrap().unwrap().id;
        db.models().mark_used(&b_id).await.unwrap();
        assert_eq!(
            db.models()
                .pick_for_role("chat")
                .await
                .unwrap()
                .unwrap()
                .name,
            "bravo"
        );
        // A "coding" request never returns the chat-only models.
        assert_eq!(
            db.models()
                .pick_for_role("coding")
                .await
                .unwrap()
                .unwrap()
                .name,
            "charlie"
        );
        let _ = a;
    }

    #[tokio::test]
    async fn set_roles_replaces_the_whole_set_and_cleans_it() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.models().insert(gguf_model("a", "ha")).await.unwrap();
        assert_eq!(a.roles, ["chat", "coding"]); // gguf_model's default roles

        let stored = db
            .models()
            .set_roles(
                &a.id,
                &[
                    "  embedding ".into(),
                    "reasoning".into(),
                    "".into(),
                    "reasoning".into(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(stored, ["embedding", "reasoning"]); // trimmed, deduped, sorted, and fully replaced

        assert_eq!(
            db.models().roles(&a.id).await.unwrap(),
            ["embedding", "reasoning"]
        );
        assert_eq!(
            db.models().get(&a.id).await.unwrap().unwrap().roles,
            ["embedding", "reasoning"]
        );

        // Re-setting replaces the whole set, doesn't accumulate.
        let replaced = db
            .models()
            .set_roles(&a.id, &["reasoning".into()])
            .await
            .unwrap();
        assert_eq!(replaced, ["reasoning"]);
        assert_eq!(db.models().roles(&a.id).await.unwrap(), ["reasoning"]);
    }

    #[tokio::test]
    async fn set_tags_cleans_and_all_tags_indexes() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.models().insert(gguf_model("a", "ha")).await.unwrap();
        let b = db.models().insert(gguf_model("b", "hb")).await.unwrap();

        let stored = db
            .models()
            .set_tags(
                &a.id,
                &[
                    "  Coding ".into(),
                    "coding".into(),
                    "".into(),
                    "Favourite".into(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(stored, ["coding", "favourite"]); // trimmed, lowered, deduped, sorted
        assert_eq!(
            db.models().tags(&a.id).await.unwrap(),
            ["coding", "favourite"]
        );

        db.models()
            .set_tags(&b.id, &["coding".into()])
            .await
            .unwrap();
        let all = db.models().all_tags().await.unwrap();
        assert_eq!(all.get(&a.id).unwrap(), &["coding", "favourite"]);
        assert_eq!(all.get(&b.id).unwrap(), &["coding"]);

        // Re-setting replaces the whole set.
        db.models().set_tags(&a.id, &["keep".into()]).await.unwrap();
        assert_eq!(db.models().tags(&a.id).await.unwrap(), ["keep"]);
    }
}
