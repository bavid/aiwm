//! Benchmark persistence (Phase 6.5). The measuring is done by [`crate::bench`];
//! this module only stores and queries.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// One finished "Test model" run, as stored.
#[derive(Debug, Clone, Serialize)]
pub struct Benchmark {
    pub id: String,
    pub model_id: String,
    pub job_id: Option<String>,
    /// `"llm"` for now; `"image"` / `"video"` later.
    pub kind: String,
    pub runs: i64,
    /// Prompt (prefill) tokens/sec, mean over the runs.
    pub prompt_tps: Option<f64>,
    /// Generation tokens/sec, mean over the runs.
    pub gen_tps: Option<f64>,
    /// Cold load time; `None` when the model was already resident.
    pub load_ms: Option<i64>,
    /// NVML VRAM peak seen during the run; `None` without a GPU.
    pub vram_peak_mb: Option<i64>,
    pub ram_peak_mb: Option<i64>,
    /// `0..1`, `1` = perfectly consistent tokens/sec across the runs.
    pub stability_score: f64,
    /// `0..100` openly-declared heuristic (speed + fit + stability), **not** a
    /// quality score (ADR-024).
    pub overall_score: i64,
    pub notes: Option<String>,
    /// Id of the [`crate::bench::suites`] suite this run used; `None` for the
    /// single-prompt "quick test" from the Model Library.
    pub suite: Option<String>,
    /// Per-prompt breakdown, already parsed from the stored JSON so API
    /// consumers get structure instead of a string; `None` without a suite (or
    /// if the stored text is not valid JSON).
    pub detail: Option<serde_json::Value>,
    pub created_at: String,
}

/// Fields a caller supplies to record a benchmark. `id` and `created_at` are set
/// here.
#[derive(Debug, Clone)]
pub struct NewBenchmark {
    pub model_id: String,
    pub job_id: Option<String>,
    pub kind: String,
    pub runs: u32,
    pub prompt_tps: Option<f64>,
    pub gen_tps: Option<f64>,
    pub load_ms: Option<u64>,
    pub vram_peak_mb: Option<u64>,
    pub ram_peak_mb: Option<u64>,
    pub stability_score: f64,
    pub overall_score: u8,
    pub notes: Option<String>,
    pub suite: Option<String>,
    /// Serialised `[{ prompt_id, tokens, gen_tps, prompt_tps }, ...]`.
    pub detail_json: Option<String>,
}

#[derive(Debug)]
pub struct BenchRepo<'a> {
    pool: &'a SqlitePool,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    model_id: String,
    job_id: Option<String>,
    kind: String,
    runs: i64,
    prompt_tps: Option<f64>,
    gen_tps: Option<f64>,
    load_ms: Option<i64>,
    vram_peak_mb: Option<i64>,
    ram_peak_mb: Option<i64>,
    stability_score: f64,
    overall_score: i64,
    notes: Option<String>,
    suite: Option<String>,
    detail_json: Option<String>,
    created_at: String,
}

impl From<Row> for Benchmark {
    fn from(r: Row) -> Self {
        let detail = r.detail_json.as_deref().and_then(|raw| {
            serde_json::from_str(raw)
                .inspect_err(|err| {
                    tracing::warn!(
                        benchmark = %r.id,
                        %err,
                        "stored benchmark detail is not valid JSON — dropping it"
                    );
                })
                .ok()
        });
        Self {
            id: r.id,
            model_id: r.model_id,
            job_id: r.job_id,
            kind: r.kind,
            runs: r.runs,
            prompt_tps: r.prompt_tps,
            gen_tps: r.gen_tps,
            load_ms: r.load_ms,
            vram_peak_mb: r.vram_peak_mb,
            ram_peak_mb: r.ram_peak_mb,
            stability_score: r.stability_score,
            overall_score: r.overall_score,
            notes: r.notes,
            suite: r.suite,
            detail,
            created_at: r.created_at,
        }
    }
}

const COLS: &str = "id, model_id, job_id, kind, runs, prompt_tps, gen_tps, load_ms, \
     vram_peak_mb, ram_peak_mb, stability_score, overall_score, notes, suite, detail_json, \
     created_at";

/// Upper bound for [`BenchRepo::list_all`], so a bad caller cannot ask for the
/// whole table.
const MAX_LIST_LIMIT: i64 = 500;

impl<'a> BenchRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Store one benchmark run.
    pub async fn insert(&self, new: NewBenchmark) -> Result<Benchmark> {
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        let as_i64 = |v: Option<u64>| v.and_then(|n| i64::try_from(n).ok());

        sqlx::query(
            "INSERT INTO benchmarks (id, model_id, job_id, kind, runs, prompt_tps, gen_tps,
                 load_ms, vram_peak_mb, ram_peak_mb, stability_score, overall_score, notes,
                 suite, detail_json, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(&id)
        .bind(&new.model_id)
        .bind(&new.job_id)
        .bind(&new.kind)
        .bind(i64::from(new.runs))
        .bind(new.prompt_tps)
        .bind(new.gen_tps)
        .bind(as_i64(new.load_ms))
        .bind(as_i64(new.vram_peak_mb))
        .bind(as_i64(new.ram_peak_mb))
        .bind(new.stability_score)
        .bind(i64::from(new.overall_score))
        .bind(&new.notes)
        .bind(&new.suite)
        .bind(&new.detail_json)
        .bind(&now)
        .execute(self.pool)
        .await?;

        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("benchmark vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Benchmark>> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM benchmarks WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Benchmark::from))
    }

    /// Every benchmark for one model, newest first.
    ///
    /// Deliberately **not** filtered by suite: a suite run is a perfectly good
    /// "latest result" for a model. Its `gen_tps` is measured at a fixed token
    /// length like the quick test's, and its `stability_score` is averaged
    /// within each prompt, so a suite row is not penalised for mixing prose and
    /// code prompts. Callers that need one suite's numbers use
    /// [`list_all`](Self::list_all) with a filter.
    pub async fn list_for(&self, model_id: &str) -> Result<Vec<Benchmark>> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM benchmarks WHERE model_id = $1 ORDER BY created_at DESC, id DESC"
        )))
        .bind(model_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Benchmark::from).collect())
    }

    /// The most recent benchmark for one model, or `None`.
    pub async fn latest_for(&self, model_id: &str) -> Result<Option<Benchmark>> {
        Ok(self.list_for(model_id).await?.into_iter().next())
    }

    /// Benchmarks across all models, newest first — the Benchmark tab's history.
    /// `suite = None` returns every row (including the suite-less quick tests);
    /// `limit` is clamped to `1..=500`.
    pub async fn list_all(&self, suite: Option<&str>, limit: i64) -> Result<Vec<Benchmark>> {
        let limit = limit.clamp(1, MAX_LIST_LIMIT);
        let filter = if suite.is_some() {
            "WHERE suite = $2"
        } else {
            ""
        };
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM benchmarks {filter} ORDER BY created_at DESC, id DESC LIMIT $1"
        )))
        .bind(limit);
        if let Some(suite) = suite {
            query = query.bind(suite);
        }
        let rows: Vec<Row> = query.fetch_all(self.pool).await?;
        Ok(rows.into_iter().map(Benchmark::from).collect())
    }

    /// The most recent benchmark for every model that has one — the Model Library
    /// score column. Keyed by `model_id`. Like [`latest_for`](Self::latest_for)
    /// this mixes suite runs and quick tests on purpose; see that method's note.
    pub async fn latest_all(&self) -> Result<Vec<Benchmark>> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM benchmarks b
             WHERE (b.created_at, b.id) = (
                 SELECT created_at, id FROM benchmarks
                 WHERE model_id = b.model_id
                 ORDER BY created_at DESC, id DESC LIMIT 1
             )
             ORDER BY b.created_at DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Benchmark::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewModel};

    async fn model(db: &Database, name: &str) -> String {
        db.models()
            .insert(NewModel {
                name: name.into(),
                format: "gguf".into(),
                file_path: format!("E:\\AI\\models\\llm\\{name}\\{name}.gguf"),
                size_bytes: 4_000 * 1024 * 1024,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }

    fn sample(model_id: &str) -> NewBenchmark {
        NewBenchmark {
            model_id: model_id.into(),
            job_id: Some("job-1".into()),
            kind: "llm".into(),
            runs: 3,
            prompt_tps: Some(420.5),
            gen_tps: Some(61.2),
            load_ms: Some(1_800),
            vram_peak_mb: Some(6_100),
            ram_peak_mb: Some(14_200),
            stability_score: 0.94,
            overall_score: 72,
            notes: None,
            suite: None,
            detail_json: None,
        }
    }

    #[tokio::test]
    async fn insert_get_round_trip() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, "qwen").await;

        let b = db.benchmarks().insert(sample(&m)).await.unwrap();
        assert_eq!(b.model_id, m);
        assert_eq!(b.runs, 3);
        assert_eq!(b.gen_tps, Some(61.2));
        assert_eq!(b.load_ms, Some(1_800));
        assert_eq!(b.overall_score, 72);

        let got = db.benchmarks().get(&b.id).await.unwrap().unwrap();
        assert_eq!(got.id, b.id);
        assert_eq!(got.vram_peak_mb, Some(6_100));
    }

    #[tokio::test]
    async fn latest_for_returns_the_newest() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, "qwen").await;

        let first = db.benchmarks().insert(sample(&m)).await.unwrap();
        let second = db
            .benchmarks()
            .insert(NewBenchmark {
                gen_tps: Some(70.0),
                ..sample(&m)
            })
            .await
            .unwrap();
        assert_ne!(first.id, second.id);

        let latest = db.benchmarks().latest_for(&m).await.unwrap().unwrap();
        assert_eq!(latest.id, second.id);
        assert_eq!(latest.gen_tps, Some(70.0));
        assert_eq!(db.benchmarks().list_for(&m).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn latest_all_is_one_row_per_model() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = model(&db, "a").await;
        let b = model(&db, "b").await;

        db.benchmarks().insert(sample(&a)).await.unwrap();
        db.benchmarks().insert(sample(&a)).await.unwrap();
        let newest_a = db
            .benchmarks()
            .insert(NewBenchmark {
                overall_score: 88,
                ..sample(&a)
            })
            .await
            .unwrap();
        db.benchmarks().insert(sample(&b)).await.unwrap();

        let all = db.benchmarks().latest_all().await.unwrap();
        assert_eq!(all.len(), 2);
        let for_a = all.iter().find(|x| x.model_id == a).unwrap();
        assert_eq!(for_a.id, newest_a.id);
        assert_eq!(for_a.overall_score, 88);
    }

    #[tokio::test]
    async fn a_suite_run_round_trips_with_its_parsed_detail() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, "qwen").await;

        let stored = db
            .benchmarks()
            .insert(NewBenchmark {
                suite: Some("chat-v1".into()),
                detail_json: Some(
                    r#"[{"prompt_id":"chat-v1-explain","tokens":256,"gen_tps":61.2,"prompt_tps":null}]"#
                        .into(),
                ),
                ..sample(&m)
            })
            .await
            .unwrap();

        assert_eq!(stored.suite.as_deref(), Some("chat-v1"));
        let detail = stored.detail.expect("parsed detail");
        assert_eq!(detail[0]["prompt_id"], "chat-v1-explain");
        assert_eq!(detail[0]["tokens"], 256);

        // A legacy row keeps both columns NULL.
        let legacy = db.benchmarks().insert(sample(&m)).await.unwrap();
        assert!(legacy.suite.is_none());
        assert!(legacy.detail.is_none());
    }

    #[tokio::test]
    async fn list_all_filters_by_suite_and_honours_the_limit() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = model(&db, "a").await;
        let b = model(&db, "b").await;

        for (model_id, suite) in [
            (&a, Some("chat-v1")),
            (&b, Some("chat-v1")),
            (&a, Some("coding-v1")),
            (&a, None),
        ] {
            db.benchmarks()
                .insert(NewBenchmark {
                    suite: suite.map(str::to_string),
                    ..sample(model_id)
                })
                .await
                .unwrap();
        }

        let all = db.benchmarks().list_all(None, 50).await.unwrap();
        assert_eq!(all.len(), 4);
        // Newest first: the last insert (the legacy row) leads.
        assert!(all[0].suite.is_none());

        let chat = db.benchmarks().list_all(Some("chat-v1"), 50).await.unwrap();
        assert_eq!(chat.len(), 2);
        assert!(chat.iter().all(|r| r.suite.as_deref() == Some("chat-v1")));

        assert_eq!(db.benchmarks().list_all(None, 2).await.unwrap().len(), 2);
        assert!(db
            .benchmarks()
            .list_all(Some("chat-v9"), 50)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn benchmarks_vanish_with_their_model() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, "qwen").await;
        db.benchmarks().insert(sample(&m)).await.unwrap();

        db.models().delete(&m).await.unwrap();
        assert!(db.benchmarks().list_for(&m).await.unwrap().is_empty());
    }
}
