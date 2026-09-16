# Training Orchestrator — Plan 1: Dataset Extensions — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make AIWM's dataset-prep pipeline the complete front half of local LoRA training: named, reusable datasets; captioning optional and pluggable (Florence-2 or a Danbooru-style tagger); filter level C (dead frames, transitions, per-clip diversity cap) with every rejection visible and reversible; per-dataset concepts assigned frame-by-frame; a guided "Learn" mode; a raw-clip mode for video models; and an export that composes captions from trigger + concept tokens + auto/hand captions in a profile-chosen order.

**Architecture:** Everything hangs off the existing `job_type=dataset_prep` pipeline (`core/src/capability/dataset/`), the `dataset_frames` table, and the Dataset tab. A `datasets` table becomes the durable object a training run will later point at; frames gain `dataset_id`, a `rejection_reason`, and clip fields. Captioners become a registry (`captioner.rs`) resolved by model-library role, exactly like `TrainingProfile` will be later; the WD tagger runs in the Python sidecar via `onnxruntime` next to Florence-2. Caption text is *composed at export time* from separately stored parts — nothing is ever baked into the raw caption. All new HTTP routes get matching Tauri commands, `ipc.ts` bindings, and `dev-mock.ts` doubles, following the existing dataset routes 1:1.

**Tech Stack:** Rust (sqlx/SQLite migrations, `image`, `image_hasher`, tokio), Python sidecar (`onnxruntime`, `numpy`, `pillow`), React 19/TS (existing Dataset tab), `ffmpeg`/`ffprobe` on PATH.

**Spec:** `docs/superpowers/specs/2026-09-16-training-orchestrator-design.md`, sections 3, 3A, 3C, 3D, 3E, 4B, 5, and implementation-order step 1. Plan 2 (trainer runtime, profiles, `training_runs`, Training tab) is written separately after this plan ships.

**Project rules that apply to every task:** TDD (write the failing test, watch it fail, then implement); `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` clean before every commit; sidecar `uv run ruff check .` + `uv run pytest -q` clean when the sidecar is touched; `pnpm typecheck`/`pnpm lint`/`pnpm build` in `ui/` when the UI is touched; never `--no-verify`; never fabricate a SHA-256 — only paste a hash you computed from a file you downloaded; commits end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Work in an isolated worktree (`superpowers:using-git-worktrees`).

---

## File map

**Create**
- `core/migrations/0015_datasets.sql` — `datasets`, `dataset_concepts`, `frame_concepts`; new columns on `dataset_frames`.
- `core/src/db/datasets.rs` — `Dataset`, `NewDataset`, `DatasetRepo`.
- `core/src/db/dataset_concepts.rs` — `DatasetConcept`, `ConceptRepo` (create/list/delete, bulk assign/unassign, per-frame map, counts).
- `core/src/capability/dataset/compose.rs` — `CaptionOrder`, `compose_caption`, `token_warning`.
- `core/src/capability/dataset/captioner.rs` — `Captioner`, `CaptionStyle`, `CAPTIONERS`, `find`, `installed`.
- `sidecar/tests/test_wd_tagger.py` — tagger JSON-RPC tests with a fake ONNX session.
- `ui/src/features/dataset/LearnSets.tsx` — the guided Learn mode.
- `ui/src/features/dataset/ConceptsPanel.tsx` — concept list, create, counts, warnings.

**Modify**
- `core/src/db/mod.rs` — register the two new repos + re-exports.
- `core/src/db/dataset.rs` — new columns on `DatasetFrame`/`NewDatasetFrame`, `list_for_dataset`, `set_rejection_reason`, `set_clip_range`.
- `core/src/capability/dataset/filter.rs` — `RejectionReason`, `is_dead_frame`, `is_transition`, `select_diverse`.
- `core/src/capability/dataset/extract.rs` — `resolve_ffprobe`, `probe_duration_secs`, `extract_preview_still`.
- `core/src/capability/dataset/caption.rs` — `caption_with(captioner, ...)`; `resolve_captioner_dir`.
- `core/src/capability/dataset/mod.rs` — request fields (`mode`, `captioner`, `max_frames_per_clip`, `min_clip_secs`), dataset row creation, rejected-frame persistence, optional captioning, clip mode, composed export.
- `core/src/model/kind.rs` — `ModelKind::WdTagger`.
- `core/src/model/catalog.rs` — two WD tagger entries.
- `core/src/api/dto.rs`, `core/src/api/handlers.rs`, `core/src/api/http.rs`, `src-tauri/src/lib.rs` — datasets, concepts, captioners, export-with-order.
- `sidecar/src/aiwm_sidecar/vision.py`, `sidecar/src/aiwm_sidecar/main.py`, `sidecar/pyproject.toml` — `tag_frame`.
- `ui/src/lib/ipc.ts`, `ui/src/lib/hooks.ts`, `ui/src/lib/dev-mock.ts`, `ui/src/features/dataset/Dataset.tsx`, `ui/src/features/dataset/dataset.css`.
- `docs/TODO.md` — dataset-pipeline entry updated.

---

### Task 1: `datasets` table, concept tables, and `DatasetRepo`

**Files:**
- Create: `core/migrations/0015_datasets.sql`
- Create: `core/src/db/datasets.rs`
- Modify: `core/src/db/mod.rs`

- [ ] **Step 1: Write the migration**

```sql
-- 0015_datasets.sql
-- A dataset is the durable object a training run (Plan 2) points at: a
-- curated set of frames or clips, reusable across many runs. Until now a
-- dataset_prep job's frames were only reachable through that job; the job
-- stays as the *producer* (prep_job_id, nulled if the job is deleted) while
-- the dataset itself survives.
CREATE TABLE datasets (
    id            TEXT PRIMARY KEY,                     -- uuid v7
    name          TEXT NOT NULL,
    mode          TEXT NOT NULL CHECK (mode IN ('frames', 'clips')),
    source_root   TEXT NOT NULL,
    trigger_word  TEXT NOT NULL DEFAULT '',             -- dataset-wide style trigger, in every caption
    prep_job_id   TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    export_dir    TEXT,                                 -- last export destination, NULL until exported
    created_at    TEXT NOT NULL
) STRICT;

-- Frames now belong to a dataset (cascade: no dataset, no frames). Rejected
-- frames are stored too, with the reason, so the curation grid can show and
-- restore them ('' = kept). Clip mode stores one row per video with its
-- preview still as frame_path and the clip itself as source_path.
ALTER TABLE dataset_frames ADD COLUMN dataset_id TEXT REFERENCES datasets(id) ON DELETE CASCADE;
ALTER TABLE dataset_frames ADD COLUMN rejection_reason TEXT NOT NULL DEFAULT '';
ALTER TABLE dataset_frames ADD COLUMN duration_secs REAL;
ALTER TABLE dataset_frames ADD COLUMN clip_start_secs REAL;
ALTER TABLE dataset_frames ADD COLUMN clip_end_secs REAL;
CREATE INDEX idx_dataset_frames_dataset ON dataset_frames(dataset_id);

-- A concept: a thing the user wants the LoRA to learn by name. `token` is
-- the trigger with no prior meaning in the base model ("kenji_xy");
-- `description` is optional extra caption text appended with it.
CREATE TABLE dataset_concepts (
    id          TEXT PRIMARY KEY,
    dataset_id  TEXT NOT NULL REFERENCES datasets(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    token       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    UNIQUE (dataset_id, token)
) STRICT;

-- Which frames show which concept. Never denormalised into the caption
-- column: export composes the final caption from these parts.
CREATE TABLE frame_concepts (
    frame_id   TEXT NOT NULL REFERENCES dataset_frames(id) ON DELETE CASCADE,
    concept_id TEXT NOT NULL REFERENCES dataset_concepts(id) ON DELETE CASCADE,
    PRIMARY KEY (frame_id, concept_id)
) STRICT;

CREATE INDEX idx_frame_concepts_concept ON frame_concepts(concept_id);
```

- [ ] **Step 2: Write the failing repo tests** in `core/src/db/datasets.rs` (create the file with only the test module and a stub type so it compiles):

```rust
//! Datasets: the durable, reusable object a training run points at. See
//! `0015_datasets.sql`.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatasetMode {
    Frames,
    Clips,
}

impl DatasetMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Clips => "clips",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "frames" => Some(Self::Frames),
            "clips" => Some(Self::Clips),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub mode: DatasetMode,
    pub source_root: String,
    pub trigger_word: String,
    pub prep_job_id: Option<String>,
    pub export_dir: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewDataset {
    pub name: String,
    pub mode: DatasetMode,
    pub source_root: String,
    pub prep_job_id: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DatasetRow {
    id: String,
    name: String,
    mode: String,
    source_root: String,
    trigger_word: String,
    prep_job_id: Option<String>,
    export_dir: Option<String>,
    created_at: String,
}

impl From<DatasetRow> for Dataset {
    fn from(r: DatasetRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            mode: DatasetMode::parse(&r.mode).unwrap_or(DatasetMode::Frames),
            source_root: r.source_root,
            trigger_word: r.trigger_word,
            prep_job_id: r.prep_job_id,
            export_dir: r.export_dir,
            created_at: r.created_at,
        }
    }
}

const SELECT_COLS: &str =
    "id, name, mode, source_root, trigger_word, prep_job_id, export_dir, created_at";

#[derive(Debug)]
pub struct DatasetRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DatasetRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewJob};

    #[tokio::test]
    async fn create_then_get_round_trips_and_defaults_trigger_to_empty() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "Anime style".into(),
                mode: DatasetMode::Frames,
                source_root: "E:\\Data\\Anime".into(),
                prep_job_id: Some(job.id.clone()),
            })
            .await
            .unwrap();
        assert_eq!(ds.mode, DatasetMode::Frames);
        assert_eq!(ds.trigger_word, "");
        assert_eq!(ds.prep_job_id.as_deref(), Some(job.id.as_str()));
        let got = db.datasets().get(&ds.id).await.unwrap().unwrap();
        assert_eq!(got, ds);
    }

    #[tokio::test]
    async fn list_is_newest_first_and_set_trigger_word_persists() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: None,
            })
            .await
            .unwrap();
        let b = db
            .datasets()
            .create(NewDataset {
                name: "B".into(),
                mode: DatasetMode::Clips,
                source_root: "y".into(),
                prep_job_id: None,
            })
            .await
            .unwrap();
        db.datasets().set_trigger_word(&a.id, " ghibli_xy ").await.unwrap();

        let all = db.datasets().list().await.unwrap();
        assert_eq!(all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec![b.id.as_str(), a.id.as_str()]);
        assert_eq!(db.datasets().get(&a.id).await.unwrap().unwrap().trigger_word, "ghibli_xy");
    }

    #[tokio::test]
    async fn deleting_the_prep_job_keeps_the_dataset_but_nulls_the_link() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job.id.clone()),
            })
            .await
            .unwrap();
        db.jobs().delete(&job.id).await.unwrap();
        let got = db.datasets().get(&ds.id).await.unwrap().unwrap();
        assert_eq!(got.prep_job_id, None);
    }

    #[tokio::test]
    async fn delete_removes_the_dataset() {
        let db = Database::connect_in_memory().await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: None,
            })
            .await
            .unwrap();
        db.datasets().delete(&ds.id).await.unwrap();
        assert!(db.datasets().get(&ds.id).await.unwrap().is_none());
    }
}
```

- [ ] **Step 3: Wire the module** in `core/src/db/mod.rs`: add `mod datasets;` (keep alphabetical: after `mod dataset;`), add `pub use datasets::{Dataset, DatasetMode, DatasetRepo, NewDataset};`, and add the accessor next to `dataset_frames`:

```rust
    pub fn datasets(&self) -> DatasetRepo<'_> {
        DatasetRepo::new(&self.pool)
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p aiwm-core --lib db::datasets`
Expected: compile error — `create`, `get`, `list`, `set_trigger_word`, `delete` not found on `DatasetRepo`.

- [ ] **Step 5: Implement the repo methods** (inside `impl<'a> DatasetRepo<'a>`):

```rust
    pub async fn create(&self, d: NewDataset) -> Result<Dataset> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO datasets (id, name, mode, source_root, prep_job_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(&d.name)
        .bind(d.mode.as_str())
        .bind(&d.source_root)
        .bind(&d.prep_job_id)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| crate::CoreError::Db("dataset vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Dataset>> {
        let row = sqlx::query_as::<_, DatasetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM datasets WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Dataset::from))
    }

    /// Newest first — the Dataset tab lists the most recent run on top.
    pub async fn list(&self) -> Result<Vec<Dataset>> {
        let rows = sqlx::query_as::<_, DatasetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM datasets ORDER BY created_at DESC, id DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Dataset::from).collect())
    }

    pub async fn set_trigger_word(&self, id: &str, trigger_word: &str) -> Result<()> {
        sqlx::query("UPDATE datasets SET trigger_word = $1 WHERE id = $2")
            .bind(trigger_word.trim())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_export_dir(&self, id: &str, export_dir: &str) -> Result<()> {
        sqlx::query("UPDATE datasets SET export_dir = $1 WHERE id = $2")
            .bind(export_dir)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM datasets WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p aiwm-core --lib db::datasets`
Expected: 4 passed. Also run `cargo test -p aiwm-core --lib db::tests` — the existing `migrations_are_idempotent` and `in_memory_database_has_the_v1_schema` tests must still pass with 0015 applied.

- [ ] **Step 7: Commit**

```bash
git add core/migrations/0015_datasets.sql core/src/db/datasets.rs core/src/db/mod.rs
git commit -m "feat(db): datasets, dataset_concepts, frame_concepts tables + DatasetRepo

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Frames belong to datasets; rejected frames and clip fields

> **Amendment after Task 1 review (2026-09-16):** the quality review proved that
> `dataset_frames.job_id ON DELETE CASCADE` (from 0014) silently deleted every
> frame of a dataset when its prep job was deleted, leaving an empty `datasets`
> row. Migration 0015 now *rebuilds* `dataset_frames` with `job_id TEXT
> REFERENCES jobs(id) ON DELETE SET NULL` (nullable) and `dataset_id ... ON
> DELETE CASCADE`: **a frame's lifecycle is governed by its dataset, not the
> producing job.** Consequences for this task: `DatasetFrame.job_id` becomes
> `Option<String>`; `NewDatasetFrame.job_id` stays `String` (always set at
> insert); `list_for_job` is unchanged; every caller that reads `frame.job_id`
> (the `GET /jobs/{id}/dataset-frames/{frame_id}/image` handler, the UI's
> `datasetFrameImageUrl(port, frame.job_id, frame.id)`) must handle `None` —
> Task 11 adds a dataset-keyed image route `GET /datasets/{id}/frames/{frame_id}/image`
> and the UI switches to it; until then treat `job_id == None` as "serve by
> frame id only". Add a test: a frame with both ids set survives job deletion
> with `job_id == None` and is removed when its dataset is deleted (Task 1
> already has this scenario in `datasets.rs`; move the assertion on the frame's
> `job_id` here once the field is optional).

**Files:**
- Modify: `core/src/db/dataset.rs`

- [ ] **Step 1: Write the failing tests** (append to the existing `mod tests` in `core/src/db/dataset.rs`):

```rust
    async fn db_with_dataset() -> (Database, String, String) {
        let (db, job_id) = db_with_job().await;
        let ds = db
            .datasets()
            .create(crate::db::NewDataset {
                name: "T".into(),
                mode: crate::db::DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job_id.clone()),
            })
            .await
            .unwrap();
        (db, job_id, ds.id)
    }

    #[tokio::test]
    async fn insert_records_dataset_rejection_and_clip_fields() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "Ghibli");
        f.dataset_id = Some(ds_id.clone());
        f.rejection_reason = "black".into();
        f.duration_secs = Some(12.5);
        let frame = db.dataset_frames().insert(f).await.unwrap();
        assert_eq!(frame.dataset_id.as_deref(), Some(ds_id.as_str()));
        assert_eq!(frame.rejection_reason, "black");
        assert_eq!(frame.duration_secs, Some(12.5));
        assert_eq!(frame.clip_start_secs, None);
    }

    #[tokio::test]
    async fn list_for_dataset_returns_only_that_datasets_frames() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut in_ds = new_frame(&job_id, "A");
        in_ds.dataset_id = Some(ds_id.clone());
        db.dataset_frames().insert(in_ds).await.unwrap();
        db.dataset_frames().insert(new_frame(&job_id, "B")).await.unwrap(); // no dataset

        let frames = db.dataset_frames().list_for_dataset(&ds_id).await.unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].tag, "A");
    }

    #[tokio::test]
    async fn set_rejection_reason_restores_a_rejected_frame() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id);
        f.rejection_reason = "blur".into();
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.dataset_frames().set_rejection_reason(&frame.id, "").await.unwrap();
        assert_eq!(db.dataset_frames().get(&frame.id).await.unwrap().unwrap().rejection_reason, "");
    }

    #[tokio::test]
    async fn set_clip_range_persists_start_and_end() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id);
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.dataset_frames().set_clip_range(&frame.id, Some(1.0), Some(4.5)).await.unwrap();
        let got = db.dataset_frames().get(&frame.id).await.unwrap().unwrap();
        assert_eq!((got.clip_start_secs, got.clip_end_secs), (Some(1.0), Some(4.5)));
    }

    #[tokio::test]
    async fn deleting_the_dataset_cascades_to_its_frames() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id.clone());
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.datasets().delete(&ds_id).await.unwrap();
        assert!(db.dataset_frames().get(&frame.id).await.unwrap().is_none());
    }
```

Also update the existing `new_frame` helper to fill the new fields:

```rust
    fn new_frame(job_id: &str, tag: &str) -> NewDatasetFrame {
        NewDatasetFrame {
            job_id: job_id.to_string(),
            dataset_id: None,
            tag: tag.to_string(),
            source_path: "E:\\Data\\Ghibli\\clip.mp4".into(),
            frame_path: "C:\\datasets\\job-1\\0001.png".into(),
            timestamp_secs: Some(1.5),
            rejection_reason: String::new(),
            duration_secs: None,
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p aiwm-core --lib db::dataset`
Expected: compile errors — unknown fields `dataset_id`/`rejection_reason`/`duration_secs`, unknown methods.

- [ ] **Step 3: Implement.** In `core/src/db/dataset.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetFrame {
    pub id: String,
    pub job_id: String,
    pub dataset_id: Option<String>,
    pub tag: String,
    pub source_path: String,
    pub frame_path: String,
    pub timestamp_secs: Option<f64>,
    pub caption: String,
    /// `"florence2"` | `"wd-eva02-tagger-v3"` | `"qwen2.5-vl"` | `""` (not captioned / hand-edited).
    pub caption_engine: String,
    pub excluded: bool,
    /// `""` = kept; otherwise one of `filter::RejectionReason::as_str()`.
    pub rejection_reason: String,
    /// Clip mode: the clip's length. `None` for a still frame.
    pub duration_secs: Option<f64>,
    pub clip_start_secs: Option<f64>,
    pub clip_end_secs: Option<f64>,
    pub created_at: String,
}
```

Extend `DatasetFrameRow` with the same five new fields (`dataset_id: Option<String>`, `rejection_reason: String`, `duration_secs: Option<f64>`, `clip_start_secs: Option<f64>`, `clip_end_secs: Option<f64>`) and the `From` impl; extend `NewDatasetFrame`:

```rust
pub struct NewDatasetFrame {
    pub job_id: String,
    pub dataset_id: Option<String>,
    pub tag: String,
    pub source_path: String,
    pub frame_path: String,
    pub timestamp_secs: Option<f64>,
    pub rejection_reason: String,
    pub duration_secs: Option<f64>,
}
```

Update `SELECT_COLS`:

```rust
const SELECT_COLS: &str = "id, job_id, dataset_id, tag, source_path, frame_path, timestamp_secs, \
     caption, caption_engine, excluded, rejection_reason, duration_secs, clip_start_secs, \
     clip_end_secs, created_at";
```

Update `insert`'s SQL to `INSERT INTO dataset_frames (id, job_id, dataset_id, tag, source_path, frame_path, timestamp_secs, rejection_reason, duration_secs, created_at) VALUES ($1..$10)` binding `f.dataset_id`, `f.rejection_reason`, `f.duration_secs` in that order. Add:

```rust
    /// Every frame of a dataset, insertion order — rejected ones included
    /// (`rejection_reason != ""`); callers filter for the curation grid.
    pub async fn list_for_dataset(&self, dataset_id: &str) -> Result<Vec<DatasetFrame>> {
        let rows = sqlx::query_as::<_, DatasetFrameRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_frames WHERE dataset_id = $1 ORDER BY created_at, id"
        )))
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(DatasetFrame::from).collect())
    }

    /// `""` restores a rejected frame into the kept set.
    pub async fn set_rejection_reason(&self, id: &str, reason: &str) -> Result<()> {
        sqlx::query("UPDATE dataset_frames SET rejection_reason = $1 WHERE id = $2")
            .bind(reason)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_clip_range(&self, id: &str, start: Option<f64>, end: Option<f64>) -> Result<()> {
        sqlx::query("UPDATE dataset_frames SET clip_start_secs = $1, clip_end_secs = $2 WHERE id = $3")
            .bind(start)
            .bind(end)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
```

Fix every other `NewDatasetFrame { .. }` literal in the crate (`capability/dataset/mod.rs` tests and `run`, `core/tests/*` if any) by adding `dataset_id: None, rejection_reason: String::new(), duration_secs: None` — `cargo build` lists each site.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p aiwm-core --lib db::dataset && cargo test -p aiwm-core --lib capability::dataset`
Expected: all pass (existing dataset tests still green).

- [ ] **Step 5: Commit**

```bash
git add core/src/db/dataset.rs core/src/capability/dataset/mod.rs
git commit -m "feat(db): dataset frames carry dataset_id, rejection_reason, clip fields

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Concept repo (create/list/delete, bulk assign, per-frame map)

**Files:**
- Create: `core/src/db/dataset_concepts.rs`
- Modify: `core/src/db/mod.rs`

- [ ] **Step 1: Write the file with types + failing tests**

```rust
//! Concepts a dataset teaches by name (spec 3A): `name` is what the user
//! sees, `token` is the trigger inserted into captions, `description` is
//! optional extra caption text. Assignment to frames is a plain join table —
//! captions are composed at export, never rewritten here.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetConcept {
    pub id: String,
    pub dataset_id: String,
    pub name: String,
    pub token: String,
    pub description: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct ConceptRow {
    id: String,
    dataset_id: String,
    name: String,
    token: String,
    description: String,
    created_at: String,
}

impl From<ConceptRow> for DatasetConcept {
    fn from(r: ConceptRow) -> Self {
        Self {
            id: r.id,
            dataset_id: r.dataset_id,
            name: r.name,
            token: r.token,
            description: r.description,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewConcept {
    pub dataset_id: String,
    pub name: String,
    pub token: String,
    pub description: String,
}

const SELECT_COLS: &str = "id, dataset_id, name, token, description, created_at";

#[derive(Debug)]
pub struct ConceptRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ConceptRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, c: NewConcept) -> Result<DatasetConcept> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO dataset_concepts (id, dataset_id, name, token, description, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(&c.dataset_id)
        .bind(c.name.trim())
        .bind(c.token.trim())
        .bind(c.description.trim())
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| crate::CoreError::Db("concept vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<DatasetConcept>> {
        let row = sqlx::query_as::<_, ConceptRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_concepts WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(DatasetConcept::from))
    }

    pub async fn list_for_dataset(&self, dataset_id: &str) -> Result<Vec<DatasetConcept>> {
        let rows = sqlx::query_as::<_, ConceptRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_concepts WHERE dataset_id = $1 ORDER BY created_at, id"
        )))
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(DatasetConcept::from).collect())
    }

    pub async fn update(&self, id: &str, name: &str, token: &str, description: &str) -> Result<()> {
        sqlx::query(
            "UPDATE dataset_concepts SET name = $1, token = $2, description = $3 WHERE id = $4",
        )
        .bind(name.trim())
        .bind(token.trim())
        .bind(description.trim())
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM dataset_concepts WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Assign `concept_id` to every frame in `frame_ids`; already-assigned
    /// pairs are ignored (idempotent), so "Alle im Set" can be clicked twice.
    pub async fn assign(&self, concept_id: &str, frame_ids: &[String]) -> Result<()> {
        for frame_id in frame_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO frame_concepts (frame_id, concept_id) VALUES ($1, $2)",
            )
            .bind(frame_id)
            .bind(concept_id)
            .execute(self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn unassign(&self, concept_id: &str, frame_ids: &[String]) -> Result<()> {
        for frame_id in frame_ids {
            sqlx::query("DELETE FROM frame_concepts WHERE frame_id = $1 AND concept_id = $2")
                .bind(frame_id)
                .bind(concept_id)
                .execute(self.pool)
                .await?;
        }
        Ok(())
    }

    /// frame_id -> concept ids, for every frame of `dataset_id` that has at
    /// least one concept (frames without concepts are simply absent).
    pub async fn map_for_dataset(&self, dataset_id: &str) -> Result<HashMap<String, Vec<String>>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT fc.frame_id, fc.concept_id FROM frame_concepts fc \
             JOIN dataset_concepts c ON c.id = fc.concept_id \
             WHERE c.dataset_id = $1 ORDER BY fc.frame_id, c.created_at",
        )
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (frame_id, concept_id) in rows {
            map.entry(frame_id).or_default().push(concept_id);
        }
        Ok(map)
    }

    /// concept_id -> number of frames carrying it (0 for concepts with none).
    pub async fn counts_for_dataset(&self, dataset_id: &str) -> Result<BTreeMap<String, usize>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT c.id, COUNT(fc.frame_id) FROM dataset_concepts c \
             LEFT JOIN frame_concepts fc ON fc.concept_id = c.id \
             WHERE c.dataset_id = $1 GROUP BY c.id",
        )
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, n)| (id, usize::try_from(n).unwrap_or(0)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, DatasetMode, NewDataset, NewDatasetFrame, NewJob};

    async fn fixture() -> (Database, String, Vec<String>) {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "T".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job.id.clone()),
            })
            .await
            .unwrap();
        let mut frame_ids = Vec::new();
        for i in 0..3 {
            let f = db
                .dataset_frames()
                .insert(NewDatasetFrame {
                    job_id: job.id.clone(),
                    dataset_id: Some(ds.id.clone()),
                    tag: "A".into(),
                    source_path: "clip.mp4".into(),
                    frame_path: format!("f{i}.png"),
                    timestamp_secs: Some(f64::from(i)),
                    rejection_reason: String::new(),
                    duration_secs: None,
                })
                .await
                .unwrap();
            frame_ids.push(f.id);
        }
        (db, ds.id, frame_ids)
    }

    #[tokio::test]
    async fn create_trims_and_rejects_duplicate_token_in_same_dataset() {
        let (db, ds_id, _) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: " Kenji ".into(),
                token: " kenji_xy ".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        assert_eq!((c.name.as_str(), c.token.as_str()), ("Kenji", "kenji_xy"));
        let dup = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id,
                name: "Other".into(),
                token: "kenji_xy".into(),
                description: "".into(),
            })
            .await;
        assert!(dup.is_err(), "UNIQUE (dataset_id, token) must reject");
    }

    #[tokio::test]
    async fn assign_is_idempotent_and_unassign_removes_only_the_pair() {
        let (db, ds_id, frames) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: "Kenji".into(),
                token: "kenji_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        db.concepts().assign(&c.id, &frames[..2]).await.unwrap();
        db.concepts().assign(&c.id, &frames[..2]).await.unwrap(); // twice, no error

        let map = db.concepts().map_for_dataset(&ds_id).await.unwrap();
        assert_eq!(map.get(&frames[0]).unwrap(), &vec![c.id.clone()]);
        assert!(map.get(&frames[2]).is_none());
        assert_eq!(db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&c.id], 2);

        db.concepts().unassign(&c.id, &frames[..1]).await.unwrap();
        assert_eq!(db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&c.id], 1);
    }

    #[tokio::test]
    async fn deleting_a_concept_cascades_its_assignments_and_counts_zero_for_unused() {
        let (db, ds_id, frames) = fixture().await;
        let used = db
            .concepts()
            .create(NewConcept { dataset_id: ds_id.clone(), name: "A".into(), token: "a_xy".into(), description: "".into() })
            .await
            .unwrap();
        let unused = db
            .concepts()
            .create(NewConcept { dataset_id: ds_id.clone(), name: "B".into(), token: "b_xy".into(), description: "".into() })
            .await
            .unwrap();
        db.concepts().assign(&used.id, &frames).await.unwrap();
        assert_eq!(db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&unused.id], 0);

        db.concepts().delete(&used.id).await.unwrap();
        assert!(db.concepts().map_for_dataset(&ds_id).await.unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Wire in `core/src/db/mod.rs`**: `mod dataset_concepts;`, `pub use dataset_concepts::{ConceptRepo, DatasetConcept, NewConcept};`, accessor `pub fn concepts(&self) -> ConceptRepo<'_> { ConceptRepo::new(&self.pool) }`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p aiwm-core --lib db::dataset_concepts`
Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add core/src/db/dataset_concepts.rs core/src/db/mod.rs
git commit -m "feat(db): dataset concepts repo with bulk assign and per-frame map

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Filter level C — dead frames, transitions, diversity cap

**Files:**
- Modify: `core/src/capability/dataset/filter.rs`

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`; reuse the existing `write_png`, `sharp_checkerboard`, `flat_gray` helpers):

```rust
    #[test]
    fn rejection_reason_round_trips_through_its_string_form() {
        for r in RejectionReason::ALL {
            assert_eq!(RejectionReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(RejectionReason::parse(""), None);
        assert_eq!(RejectionReason::parse("nonsense"), None);
    }

    #[test]
    fn is_dead_frame_flags_black_and_white_but_not_mid_gray_or_content() {
        let tmp = tempfile::tempdir().unwrap();
        let black = tmp.path().join("black.png");
        let white = tmp.path().join("white.png");
        let gray = tmp.path().join("gray.png");
        let content = tmp.path().join("content.png");
        write_png(&black, &flat_gray(32, 3));
        write_png(&white, &flat_gray(32, 252));
        write_png(&gray, &flat_gray(32, 128));
        write_png(&content, &sharp_checkerboard(32));

        assert!(is_dead_frame(&black).unwrap());
        assert!(is_dead_frame(&white).unwrap());
        assert!(!is_dead_frame(&gray).unwrap(), "flat mid-gray is blurry, not dead");
        assert!(!is_dead_frame(&content).unwrap());
    }

    #[test]
    fn is_transition_needs_blur_and_two_distant_neighbors() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        write_png(&a, &sharp_checkerboard(32));
        write_png(&b, &flat_gray(32, 10));
        let ha = phash_of(&a).unwrap();
        let hb = phash_of(&b).unwrap();
        let far = ha.dist(&hb);
        assert!(far > DEFAULT_TRANSITION_MIN_DISTANCE, "fixture must be distant: {far}");

        // Blurry and far from both neighbours -> transition.
        assert!(is_transition(true, Some(&ha), &hb, Some(&ha), DEFAULT_TRANSITION_MIN_DISTANCE));
        // Sharp -> never a transition, however different the neighbours are.
        assert!(!is_transition(false, Some(&ha), &hb, Some(&ha), DEFAULT_TRANSITION_MIN_DISTANCE));
        // Blurry but similar to one neighbour -> not a transition.
        assert!(!is_transition(true, Some(&hb), &hb, Some(&ha), DEFAULT_TRANSITION_MIN_DISTANCE));
        // First/last frame (a missing neighbour) is never a transition.
        assert!(!is_transition(true, None, &hb, Some(&ha), DEFAULT_TRANSITION_MIN_DISTANCE));
    }

    #[test]
    fn select_diverse_keeps_everything_under_the_cap_and_prefers_spread_above_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut hashes = Vec::new();
        // 3 near-identical checkerboards, 1 flat dark, 1 flat light.
        for i in 0..3 {
            let p = tmp.path().join(format!("c{i}.png"));
            write_png(&p, &sharp_checkerboard(32));
            hashes.push(phash_of(&p).unwrap());
        }
        let dark = tmp.path().join("dark.png");
        let light = tmp.path().join("light.png");
        write_png(&dark, &flat_gray(32, 10));
        write_png(&light, &flat_gray(32, 240));
        hashes.push(phash_of(&dark).unwrap());
        hashes.push(phash_of(&light).unwrap());

        assert_eq!(select_diverse(&hashes, 0), (0..5).collect::<Vec<_>>(), "0 = unlimited");
        assert_eq!(select_diverse(&hashes, 10), (0..5).collect::<Vec<_>>());

        let picked = select_diverse(&hashes, 2);
        assert_eq!(picked.len(), 2);
        assert!(picked.contains(&0), "always starts with the first frame");
        assert!(picked.contains(&3) || picked.contains(&4), "second pick is the most different");
        let mut sorted = picked.clone();
        sorted.sort_unstable();
        assert_eq!(picked, sorted, "returned in original order");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p aiwm-core --lib capability::dataset::filter`
Expected: compile errors — `RejectionReason`, `is_dead_frame`, `is_transition`, `select_diverse`, `DEFAULT_TRANSITION_MIN_DISTANCE` not found.

- [ ] **Step 3: Implement** (add to `filter.rs`, after the existing constants):

```rust
/// Why a frame was dropped. Stored on the row (`dataset_frames.rejection_reason`)
/// so the curation grid can show each reason as a filter chip and restore
/// individual frames. `""` on the row means "kept".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    Black,
    Transition,
    Blur,
    Duplicate,
    Cap,
    /// Clip mode only: undecodable or shorter than the minimum.
    Unusable,
}

impl RejectionReason {
    pub const ALL: [RejectionReason; 6] = [
        Self::Black,
        Self::Transition,
        Self::Blur,
        Self::Duplicate,
        Self::Cap,
        Self::Unusable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Transition => "transition",
            Self::Blur => "blur",
            Self::Duplicate => "duplicate",
            Self::Cap => "cap",
            Self::Unusable => "unusable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

/// A frame whose mean luma sits within this many levels of pure black (0) or
/// pure white (255) *and* whose pixels barely vary is a fade/blank frame.
const DEAD_FRAME_LUMA_MARGIN: f64 = 12.0;
const DEAD_FRAME_MAX_STDDEV: f64 = 6.0;

/// Hamming distance above which two frames count as "different pictures"
/// for transition detection — deliberately far above the duplicate
/// threshold (6): a cut between two scenes is *very* different, a slow pan
/// is not.
pub const DEFAULT_TRANSITION_MIN_DISTANCE: u32 = 20;

/// Per-clip diversity cap default (spec 3, filter 4). `0` = unlimited.
pub const DEFAULT_MAX_FRAMES_PER_CLIP: usize = 40;

fn luma_mean_and_stddev(gray: &image::GrayImage) -> (f64, f64) {
    let n = (gray.width() as f64) * (gray.height() as f64);
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = gray.pixels().map(|p| f64::from(p.0[0])).sum::<f64>() / n;
    let var = gray
        .pixels()
        .map(|p| {
            let d = f64::from(p.0[0]) - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    (mean, var.sqrt())
}

/// (Nearly) all-black or all-white with almost no variation — a fade, a
/// blank, a dead frame between scenes.
pub fn is_dead_frame(path: &Path) -> Result<bool> {
    let img = image::open(path)
        .map_err(|e| dataset_err(format!("read {} for dead-frame check: {e}", path.display())))?;
    let (mean, stddev) = luma_mean_and_stddev(&img.to_luma8());
    let near_black = mean <= DEAD_FRAME_LUMA_MARGIN;
    let near_white = mean >= 255.0 - DEAD_FRAME_LUMA_MARGIN;
    Ok((near_black || near_white) && stddev <= DEAD_FRAME_MAX_STDDEV)
}

/// A transition (cut smear / cross-fade) is a *blurry* frame that is very
/// different from both its previous and its next neighbour. Either
/// neighbour missing (first/last frame) means "not a transition".
pub fn is_transition(
    blurry: bool,
    prev: Option<&ImageHash>,
    this: &ImageHash,
    next: Option<&ImageHash>,
    min_distance: u32,
) -> bool {
    let (Some(prev), Some(next)) = (prev, next) else {
        return false;
    };
    blurry && prev.dist(this) > min_distance && next.dist(this) > min_distance
}

/// Farthest-point selection: keep at most `cap` of `hashes` (0 = all),
/// always starting with index 0, then repeatedly the frame whose *minimum*
/// distance to everything already chosen is largest. Returns indices in
/// their original order.
pub fn select_diverse(hashes: &[ImageHash], cap: usize) -> Vec<usize> {
    if cap == 0 || hashes.len() <= cap {
        return (0..hashes.len()).collect();
    }
    let mut chosen: Vec<usize> = vec![0];
    let mut min_dist: Vec<u32> = hashes.iter().map(|h| hashes[0].dist(h)).collect();
    while chosen.len() < cap {
        let (best, _) = min_dist
            .iter()
            .enumerate()
            .filter(|(i, _)| !chosen.contains(i))
            .max_by_key(|(i, d)| (**d, std::cmp::Reverse(*i)))
            .expect("fewer chosen than hashes");
        chosen.push(best);
        for (i, d) in min_dist.iter_mut().enumerate() {
            *d = (*d).min(hashes[best].dist(&hashes[i]));
        }
    }
    chosen.sort_unstable();
    chosen
}
```

(`max_by_key` with `Reverse(i)` breaks ties toward the earlier frame, keeping selection deterministic.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p aiwm-core --lib capability::dataset::filter`
Expected: all pass, including the pre-existing blur/dedup tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/capability/dataset/filter.rs
git commit -m "feat(dataset): rejection reasons, dead-frame and transition filters, diversity cap

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Caption composition and token check (pure functions)

**Files:**
- Create: `core/src/capability/dataset/compose.rs`
- Modify: `core/src/capability/dataset/mod.rs` (add `mod compose;` + `pub use compose::{CaptionOrder, CaptionStyle, compose_caption, token_warning};`)

- [ ] **Step 1: Write the file with failing tests**

```rust
//! Caption composition (spec 3A/3D): the exported caption is assembled from
//! separately stored parts — the dataset trigger, each assigned concept's
//! token (+ optional description), and the frame's own auto/hand caption —
//! in an order the training profile chooses (tags first for Anime/SDXL,
//! prose first for FLUX.2). Nothing here is ever written back into
//! `dataset_frames.caption`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionOrder {
    TagsFirst,
    ProseFirst,
}

/// What the auto caption is: a comma-separated tag list (WD tagger) or a
/// sentence (Florence-2 / JoyCaption). Decides whether it joins with ", "
/// as one more tag block or is kept as its own clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionStyle {
    Prose,
    Tags,
}

/// One assigned concept as it reaches composition.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptPart<'a> {
    pub token: &'a str,
    pub description: &'a str,
}

/// Build the final caption. Rules (all covered by the tests below):
/// - the trigger (if non-empty) always comes first;
/// - concept tokens follow, each as `token` or `token, description`;
/// - the auto/hand caption is placed after the tags (`TagsFirst`) or the
///   trigger+tokens are placed after it (`ProseFirst`);
/// - empty parts are skipped; the result never has leading/trailing
///   separators.
pub fn compose_caption(
    order: CaptionOrder,
    trigger: &str,
    concepts: &[ConceptPart<'_>],
    auto_caption: &str,
    _auto_style: CaptionStyle,
) -> String {
    let mut tag_block: Vec<String> = Vec::new();
    let trigger = trigger.trim();
    if !trigger.is_empty() {
        tag_block.push(trigger.to_string());
    }
    for c in concepts {
        let token = c.token.trim();
        if token.is_empty() {
            continue;
        }
        let description = c.description.trim();
        if description.is_empty() {
            tag_block.push(token.to_string());
        } else {
            tag_block.push(format!("{token}, {description}"));
        }
    }
    let tags = tag_block.join(", ");
    let prose = auto_caption.trim();
    match (tags.is_empty(), prose.is_empty(), order) {
        (true, true, _) => String::new(),
        (false, true, _) => tags,
        (true, false, _) => prose.to_string(),
        (false, false, CaptionOrder::TagsFirst) => format!("{tags}, {prose}"),
        (false, false, CaptionOrder::ProseFirst) => format!("{prose}, {tags}"),
    }
}

/// Words a trigger token must not be: they already mean something to the
/// base model, so training on them drags the whole concept along. Lowercase.
const COMMON_WORDS: &[&str] = &[
    "anime", "manga", "photo", "picture", "image", "art", "style", "character", "person", "man",
    "woman", "boy", "girl", "cat", "dog", "foot", "feet", "hand", "hands", "face", "eyes",
    "hair", "dress", "shirt", "car", "house", "tree", "sky", "night", "day", "red", "blue",
    "green", "black", "white",
];

/// `Some(warning)` when `token` looks like an ordinary word (or is empty /
/// contains spaces). The warning includes a concrete suggestion.
pub fn token_warning(token: &str) -> Option<String> {
    let t = token.trim();
    if t.is_empty() {
        return Some("Token is empty — use something like 'kenji_xy'.".into());
    }
    if t.chars().any(char::is_whitespace) {
        return Some(format!(
            "Token contains spaces — use one word, e.g. '{}_xy'.",
            t.split_whitespace().collect::<Vec<_>>().join("_").to_lowercase()
        ));
    }
    let lower = t.to_lowercase();
    if COMMON_WORDS.contains(&lower.as_str()) {
        return Some(format!(
            "'{t}' is an ordinary word the base model already knows — use a made-up token like '{lower}_xy'."
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part<'a>(token: &'a str, description: &'a str) -> ConceptPart<'a> {
        ConceptPart { token, description }
    }

    #[test]
    fn tags_first_puts_trigger_then_tokens_then_prose() {
        let out = compose_caption(
            CaptionOrder::TagsFirst,
            "ghibli_xy",
            &[part("kenji_xy", "bare, visible"), part("pusemukkel_xy", "")],
            "a boy running across a field",
            CaptionStyle::Prose,
        );
        assert_eq!(out, "ghibli_xy, kenji_xy, bare, visible, pusemukkel_xy, a boy running across a field");
    }

    #[test]
    fn prose_first_puts_the_caption_before_trigger_and_tokens() {
        let out = compose_caption(
            CaptionOrder::ProseFirst,
            "ghibli_xy",
            &[part("kenji_xy", "")],
            "a boy running across a field",
            CaptionStyle::Prose,
        );
        assert_eq!(out, "a boy running across a field, ghibli_xy, kenji_xy");
    }

    #[test]
    fn without_a_caption_only_trigger_and_tokens_remain_in_stable_order() {
        let out = compose_caption(
            CaptionOrder::ProseFirst,
            "ghibli_xy",
            &[part("b_xy", ""), part("a_xy", "")],
            "",
            CaptionStyle::Tags,
        );
        assert_eq!(out, "ghibli_xy, b_xy, a_xy");
    }

    #[test]
    fn everything_empty_yields_an_empty_caption_and_blank_parts_are_skipped() {
        assert_eq!(compose_caption(CaptionOrder::TagsFirst, " ", &[part("  ", "")], "  ", CaptionStyle::Prose), "");
        assert_eq!(compose_caption(CaptionOrder::TagsFirst, "", &[], "just prose", CaptionStyle::Prose), "just prose");
    }

    #[test]
    fn token_warning_flags_common_words_spaces_and_empty_but_not_made_up_tokens() {
        assert!(token_warning("anime").unwrap().contains("anime_xy"));
        assert!(token_warning("Foot").unwrap().contains("foot_xy"));
        assert!(token_warning("old man").unwrap().contains("old_man_xy"));
        assert!(token_warning("").is_some());
        assert_eq!(token_warning("kenji_xy"), None);
        assert_eq!(token_warning("pusemukkel"), None);
    }
}
```

- [ ] **Step 2: Run tests** — `cargo test -p aiwm-core --lib capability::dataset::compose` → 5 passed (the implementation is in the same file; if any fails, fix the implementation, not the test).

- [ ] **Step 3: Commit**

```bash
git add core/src/capability/dataset/compose.rs core/src/capability/dataset/mod.rs
git commit -m "feat(dataset): caption composition from trigger, concept tokens and caption; token check

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Captioner registry, `ModelKind::WdTagger`, catalog entries

**Files:**
- Create: `core/src/capability/dataset/captioner.rs`
- Modify: `core/src/capability/dataset/mod.rs` (`mod captioner;` + `pub use captioner::{Captioner, CaptionerStatus, CAPTIONERS, FLORENCE2_ID, WD_TAGGER_ID};`)
- Modify: `core/src/model/kind.rs`, `core/src/model/catalog.rs`

- [ ] **Step 1: Add `ModelKind::WdTagger`** in `core/src/model/kind.rs`, mirroring `DiaEngine` (a directory of sibling files consumed by the sidecar):

```rust
    /// SmilingWolf's WD Danbooru tagger — `model.onnx` + `selected_tags.csv`,
    /// co-located, consumed by the Python sidecar (`vision.tag_frame`).
    /// Never a ComfyUI model.
    WdTagger,
```

and in each `match`: `accepts_ext`: `Self::WdTagger => ext == "onnx" || ext == "csv",`; `default_role`: `Self::WdTagger => Some("vision_wd_tagger"),`; `store_subdir`: `Self::WdTagger => "vision/wd-tagger",`; `comfy_folder`: add `Self::WdTagger` to the `return None` arm; `as_str`: `Self::WdTagger => "wd_tagger",`; `parse` (the `"lora" | "loras" => ...` style match): `"wd_tagger" => Self::WdTagger,`. Add to the kind's `ALL`-style list if one exists (line ~194 area). Add a test next to the existing `default_role` test:

```rust
    #[test]
    fn wd_tagger_is_a_sidecar_kind_with_its_own_folder() {
        assert_eq!(ModelKind::WdTagger.default_role(), Some("vision_wd_tagger"));
        assert_eq!(ModelKind::WdTagger.store_subdir(), "vision/wd-tagger");
        assert_eq!(ModelKind::WdTagger.comfy_folder(), None);
        assert!(ModelKind::WdTagger.accepts_ext("onnx"));
        assert!(ModelKind::WdTagger.accepts_ext("csv"));
        assert!(!ModelKind::WdTagger.accepts_ext("safetensors"));
    }
```

Run `cargo test -p aiwm-core --lib model::kind` → passes.

- [ ] **Step 2: Compute the real hashes** (never type a hash you did not compute). In the scratchpad directory:

```bash
curl -sSL -o model.onnx "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/main/model.onnx"
curl -sSL -o selected_tags.csv "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/main/selected_tags.csv"
sha256sum model.onnx selected_tags.csv
stat -c "%n %s" model.onnx selected_tags.csv
```

Paste the printed sha256 and byte sizes into the two catalog entries below. Keep the two files: Task 7's real-run test uses them.

- [ ] **Step 3: Add the catalog entries** at the end of `KNOWN_MODELS` in `core/src/model/catalog.rs` (use the values from Step 2):

```rust
    KnownModel {
        id: "wd-eva02-large-tagger-v3-model",
        name: "WD EVA02-Large Tagger v3 — model",
        kind: "wd_tagger",
        family: None,
        publisher: "SmilingWolf",
        repo: "SmilingWolf/wd-eva02-large-tagger-v3",
        file: "model.onnx",
        url: "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/main/model.onnx",
        sha256: "<paste the sha256sum output for model.onnx>",
        size_bytes: 0, // <paste the stat byte size for model.onnx>
        license: "Apache-2.0",
        note: "Danbooru-style tag captioner (rating, character and general tags, explicit \
               tags included) for anime/illustration datasets. 0.3B parameters, runs on the \
               CPU via onnxruntime. Needs the tag list below in the same folder.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "wd-eva02-large-tagger-v3-tags",
        name: "WD EVA02-Large Tagger v3 — tag list",
        kind: "wd_tagger",
        family: None,
        publisher: "SmilingWolf",
        repo: "SmilingWolf/wd-eva02-large-tagger-v3",
        file: "selected_tags.csv",
        url: "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/main/selected_tags.csv",
        sha256: "<paste the sha256sum output for selected_tags.csv>",
        size_bytes: 0, // <paste the stat byte size for selected_tags.csv>
        license: "Apache-2.0",
        note: "The tag vocabulary the tagger's outputs map onto. Required companion of the \
               model above.",
        is_default: false,
        media: "image",
    },
```

(The `<paste …>` markers exist only in this plan so you cannot forget; the committed file must contain the real values.) Run `cargo test -p aiwm-core --lib model::catalog` — the existing catalog consistency tests (unique ids, url ends with file, known kinds) must pass; if a test asserts every `media: "image"` entry's kind is a ComfyUI kind, extend that test to allow `wd_tagger`.

- [ ] **Step 4: Write the registry with failing tests** in `core/src/capability/dataset/captioner.rs`:

```rust
//! Captioner registry (spec 3D): Florence-2 is one entry, not the pipeline.
//! Each captioner is resolved from the model library by role, exactly like
//! `caption::resolve_florence2_dir` always did; `installed` is what the
//! Dataset tab's "Beschreiben mit" dropdown filters on.

use serde::Serialize;

use super::compose::CaptionStyle;
use crate::db::Database;
use crate::Result;

pub const FLORENCE2_ID: &str = "florence2";
pub const WD_TAGGER_ID: &str = "wd-eva02-tagger-v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Captioner {
    pub id: &'static str,
    pub name: &'static str,
    pub style: CaptionStyle,
    /// Model-library role its files are imported under.
    pub role: &'static str,
    pub vram_mb: u64,
    pub license: &'static str,
    /// Whether the frame-X-vs-X+N temporal escalation applies on top of it.
    /// A tagger has no sentence to judge for confidence, so it cannot.
    pub supports_escalation: bool,
}

pub const CAPTIONERS: &[Captioner] = &[
    Captioner {
        id: FLORENCE2_ID,
        name: "Florence-2 (prose)",
        style: CaptionStyle::Prose,
        role: super::caption::FLORENCE2_ROLE,
        vram_mb: super::caption::FLORENCE2_VRAM_FALLBACK_MB,
        license: "MIT",
        supports_escalation: true,
    },
    Captioner {
        id: WD_TAGGER_ID,
        name: "WD EVA02 Tagger v3 (Danbooru tags)",
        style: CaptionStyle::Tags,
        role: "vision_wd_tagger",
        vram_mb: 0, // onnxruntime on the CPU
        license: "Apache-2.0",
        supports_escalation: false,
    },
];

pub fn find(id: &str) -> Option<&'static Captioner> {
    CAPTIONERS.iter().find(|c| c.id == id)
}

/// A registry entry plus whether its files are in the library — what the
/// UI lists.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaptionerStatus {
    #[serde(flatten)]
    pub captioner: Captioner,
    pub installed: bool,
}

pub async fn statuses(db: &Database) -> Result<Vec<CaptionerStatus>> {
    let mut out = Vec::with_capacity(CAPTIONERS.len());
    for c in CAPTIONERS {
        let installed = !db.models().for_role(c.role).await?.is_empty();
        out.push(CaptionerStatus { captioner: *c, installed });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;

    #[test]
    fn ids_are_unique_and_find_works() {
        let mut ids: Vec<&str> = CAPTIONERS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CAPTIONERS.len());
        assert_eq!(find(WD_TAGGER_ID).unwrap().style, CaptionStyle::Tags);
        assert!(find("nope").is_none());
    }

    #[test]
    fn only_prose_captioners_support_escalation() {
        for c in CAPTIONERS {
            assert_eq!(c.supports_escalation, c.style == CaptionStyle::Prose, "{}", c.id);
        }
    }

    #[tokio::test]
    async fn statuses_reflect_which_roles_are_present_in_the_library() {
        let db = Database::connect_in_memory().await.unwrap();
        db.models()
            .insert(NewModel {
                name: "model.onnx".into(),
                format: "onnx".into(),
                file_path: "E:\\AI\\models\\vision\\wd-tagger\\model.onnx".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec!["vision_wd_tagger".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        let s = statuses(&db).await.unwrap();
        let by_id = |id: &str| s.iter().find(|x| x.captioner.id == id).unwrap().installed;
        assert!(by_id(WD_TAGGER_ID));
        assert!(!by_id(FLORENCE2_ID));
    }
}
```

- [ ] **Step 5: Run** `cargo test -p aiwm-core --lib capability::dataset::captioner` → 3 passed.

- [ ] **Step 6: Commit**

```bash
git add core/src/capability/dataset/captioner.rs core/src/capability/dataset/mod.rs core/src/model/kind.rs core/src/model/catalog.rs
git commit -m "feat(dataset): captioner registry; WD EVA02 tagger as a library kind with catalog entries

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Sidecar `tag_frame` (WD tagger via onnxruntime)

**Files:**
- Modify: `sidecar/pyproject.toml` (add `"onnxruntime>=1.17"` to `dependencies`)
- Modify: `sidecar/src/aiwm_sidecar/vision.py`
- Modify: `sidecar/src/aiwm_sidecar/main.py` (dispatch + `CAPABILITIES`)
- Create: `sidecar/tests/test_wd_tagger.py`

Facts this task relies on (verified against the model card and the reference tagger implementations in Task 6's research): the WD v3 taggers take a 448×448 image, **BGR** channel order, float32 0–255 range, NHWC; `selected_tags.csv` has columns `tag_id,name,category,count` with category `9` = rating, `4` = character, `0` = general; outputs are sigmoid probabilities aligned to the CSV row order.

- [ ] **Step 1: Write the failing tests** in `sidecar/tests/test_wd_tagger.py`:

```python
"""`tag_frame` -- WD Danbooru tagger for the dataset-prep pipeline. The real
ONNX model (~1.2 GB) is never loaded here: `vision._load_wd_tagger` is
monkeypatched with a fake session/tag table, the same way
`test_vision_captioning.py` fakes Florence-2."""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest
from PIL import Image

from aiwm_sidecar import main, vision


class FakeWdTagger:
    """Stands in for `vision._WdTaggerEngine`: 6 tags, fixed probabilities."""

    def __init__(self) -> None:
        self.tag_names = ["general", "sensitive", "explicit", "kenji", "1boy", "outdoors"]
        self.categories = [9, 9, 9, 4, 0, 0]
        self.calls: list[dict[str, object]] = []

    def predict(self, image_path: str) -> np.ndarray:
        self.calls.append({"image_path": image_path})
        return np.array([0.10, 0.20, 0.85, 0.70, 0.90, 0.30], dtype=np.float32)


@pytest.fixture(autouse=True)
def clear_cache():
    vision._wd_tagger_cache.clear()
    yield
    vision._wd_tagger_cache.clear()


@pytest.fixture
def image_file(tmp_path: Path) -> str:
    p = tmp_path / "frame.png"
    Image.new("RGB", (8, 8), (200, 30, 30)).save(p)
    return str(p)


@pytest.fixture
def model_dir(tmp_path: Path) -> str:
    d = tmp_path / "wd-tagger"
    d.mkdir()
    (d / "model.onnx").write_bytes(b"not a real model")
    (d / "selected_tags.csv").write_text("tag_id,name,category,count\n", encoding="utf-8")
    return str(d)


def test_tag_frame_returns_general_and_character_tags_above_threshold(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    fake = FakeWdTagger()
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: fake)

    result = vision.tag_frame(
        {"image_path": image_file, "model_dir": model_dir, "threshold": 0.5}
    )

    assert result["engine"] == "wd-eva02-tagger-v3"
    assert result["caption"] == "kenji, 1boy"
    assert result["rating"] == "explicit"
    assert fake.calls[0]["image_path"] == image_file


def test_tag_frame_threshold_default_is_the_model_cards_and_underscores_become_spaces(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    fake = FakeWdTagger()
    fake.tag_names[5] = "blue_sky"
    fake.predict = lambda _p: np.array([0.0, 0.0, 0.0, 0.0, 0.0, 0.9], dtype=np.float32)  # type: ignore[method-assign]
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: fake)

    result = vision.tag_frame({"image_path": image_file, "model_dir": model_dir})

    assert result["caption"] == "blue sky"
    assert result["threshold"] == vision._WD_DEFAULT_THRESHOLD


def test_tag_frame_validates_its_params(image_file: str, model_dir: str) -> None:
    with pytest.raises(ValueError, match="image_path"):
        vision.tag_frame({"model_dir": model_dir})
    with pytest.raises(ValueError, match="model_dir"):
        vision.tag_frame({"image_path": image_file})
    with pytest.raises(ValueError, match="not found"):
        vision.tag_frame({"image_path": "C:/nope.png", "model_dir": model_dir})


def test_tag_frame_is_dispatched_over_json_rpc(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: FakeWdTagger())
    response = main.handle_request(
        {
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tag_frame",
            "params": {"image_path": image_file, "model_dir": model_dir},
        }
    )
    assert response["result"]["caption"] == "kenji, 1boy"
    assert "tag_frame" in main.CAPABILITIES


def test_preprocess_produces_448_bgr_float_batch(image_file: str) -> None:
    batch = vision._wd_preprocess(image_file, 448)
    assert batch.shape == (1, 448, 448, 3)
    assert batch.dtype == np.float32
    # Source pixel was (R=200, G=30, B=30); BGR order means channel 0 is blue.
    assert batch[0, 224, 224, 0] == pytest.approx(30.0)
    assert batch[0, 224, 224, 2] == pytest.approx(200.0)
```

(If `main.handle_request` is not the actual name of the sidecar's request dispatcher, use the function the existing `test_vision_captioning.py` dispatch test calls — search that file for `"method": "caption_frame"` and mirror it exactly.)

- [ ] **Step 2: Run** `cd sidecar && uv run pytest tests/test_wd_tagger.py -q` → fails: `vision` has no attribute `_wd_tagger_cache` / `tag_frame`.

- [ ] **Step 3: Implement** — append to `vision.py`:

```python
# --- WD Danbooru tagger (SmilingWolf, Apache-2.0) ---------------------------
#
# A 0.3B ONNX classifier over the Danbooru tag vocabulary. Runs on the CPU
# via onnxruntime; no torch involved. Input contract (from the reference
# tagger implementations for the v3 models): 448x448, BGR, float32 0..255,
# NHWC; outputs are sigmoid probabilities in `selected_tags.csv` row order.

_WD_DEFAULT_THRESHOLD = 0.35
_WD_RATING_CATEGORY = 9
_WD_CHARACTER_CATEGORY = 4
_WD_GENERAL_CATEGORY = 0


def _wd_preprocess(image_path: str, size: int) -> "np.ndarray":
    import numpy as np
    from PIL import Image

    image = Image.open(image_path).convert("RGBA")
    # White background (Danbooru images are padded on white), square pad.
    canvas = Image.new("RGBA", image.size, (255, 255, 255, 255))
    canvas.alpha_composite(image)
    rgb = canvas.convert("RGB")
    w, h = rgb.size
    side = max(w, h)
    square = Image.new("RGB", (side, side), (255, 255, 255))
    square.paste(rgb, ((side - w) // 2, (side - h) // 2))
    resized = square.resize((size, size), Image.Resampling.BICUBIC)
    arr = np.asarray(resized, dtype=np.float32)[:, :, ::-1]  # RGB -> BGR
    return np.expand_dims(arr, 0)


class _WdTaggerEngine:
    def __init__(self, session: Any, tag_names: list[str], categories: list[int], size: int) -> None:
        self._session = session
        self.tag_names = tag_names
        self.categories = categories
        self._size = size
        self._input_name = session.get_inputs()[0].name

    def predict(self, image_path: str) -> "np.ndarray":
        batch = _wd_preprocess(image_path, self._size)
        outputs = self._session.run(None, {self._input_name: batch})
        return outputs[0][0]


_wd_tagger_cache: dict[str, _WdTaggerEngine] = {}


def _construct_wd_tagger(model_dir: str) -> _WdTaggerEngine:
    import csv

    import onnxruntime as ort

    model_path = Path(model_dir) / "model.onnx"
    tags_path = Path(model_dir) / "selected_tags.csv"
    _require_existing_file(str(model_path), "WD tagger model")
    _require_existing_file(str(tags_path), "WD tagger tag list")

    session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
    size = int(session.get_inputs()[0].shape[1])  # NHWC: [1, H, W, 3]
    tag_names: list[str] = []
    categories: list[int] = []
    with tags_path.open(encoding="utf-8", newline="") as fh:
        for row in csv.DictReader(fh):
            tag_names.append(row["name"])
            categories.append(int(row["category"]))
    return _WdTaggerEngine(session, tag_names, categories, size)


def _load_wd_tagger(model_dir: str) -> _WdTaggerEngine:
    if model_dir not in _wd_tagger_cache:
        _wd_tagger_cache[model_dir] = _construct_wd_tagger(model_dir)
    return _wd_tagger_cache[model_dir]


def tag_frame(params: dict[str, Any]) -> dict[str, Any]:
    """WD tagger, single image. JSON-RPC `tag_frame`. Returns the general +
    character tags above `threshold` as one comma-separated caption (Danbooru
    underscores become spaces), plus the top rating tag separately."""
    image_path = str(params.get("image_path") or "")
    model_dir = str(params.get("model_dir") or "")
    threshold = float(params.get("threshold") or _WD_DEFAULT_THRESHOLD)
    if not image_path:
        raise _vision_value_error("`image_path` is required")
    if not model_dir:
        raise _vision_value_error("`model_dir` is required")
    _require_existing_file(image_path, "image")
    _require_dir(model_dir, "WD tagger model")

    engine = _load_wd_tagger(model_dir)
    probs = engine.predict(image_path)

    rating = ""
    rating_prob = -1.0
    tags: list[str] = []
    for name, category, prob in zip(engine.tag_names, engine.categories, probs, strict=True):
        p = float(prob)
        if category == _WD_RATING_CATEGORY:
            if p > rating_prob:
                rating, rating_prob = name, p
        elif category in (_WD_CHARACTER_CATEGORY, _WD_GENERAL_CATEGORY) and p >= threshold:
            tags.append(name.replace("_", " "))
    return {
        "caption": ", ".join(tags),
        "rating": rating,
        "threshold": threshold,
        "engine": "wd-eva02-tagger-v3",
    }
```

Add `import numpy as np` is **not** needed at module top (kept lazy inside functions to keep the import-light module property; the type hints use string annotations). In `main.py`: add `"tag_frame"` to `CAPABILITIES` (line ~31 list) and a dispatch branch identical to `caption_frame`'s but importing/calling `tag_frame` with error text `"tagging failed: {e}"`. Add `"onnxruntime>=1.17",` to `pyproject.toml` dependencies and run `uv sync`.

- [ ] **Step 4: Run** `cd sidecar && uv run pytest -q && uv run ruff check .` → all pass (existing 77 + 5 new).

- [ ] **Step 5: One real run against the actual ONNX model** (the two files from Task 6 Step 2, no GPU needed):

```bash
cd sidecar && uv run python -c "
from aiwm_sidecar import vision
r = vision.tag_frame({'image_path': '<any real anime frame or screenshot png on disk>', 'model_dir': '<folder containing model.onnx + selected_tags.csv>'})
print(r)"
```

Expected: a non-empty comma-separated caption and a rating in `{general, sensitive, questionable, explicit}`. If the input tensor name/shape assumption fails, `session.get_inputs()[0]` tells you the real one — fix `_construct_wd_tagger`, never the test. Record the observed input name/shape in the module docstring.

- [ ] **Step 6: Commit**

```bash
git add sidecar/pyproject.toml sidecar/uv.lock sidecar/src/aiwm_sidecar/vision.py sidecar/src/aiwm_sidecar/main.py sidecar/tests/test_wd_tagger.py
git commit -m "feat(sidecar): tag_frame — WD Danbooru tagger via onnxruntime

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: `caption_with` — captioner-aware dispatch in `caption.rs`

**Files:**
- Modify: `core/src/capability/dataset/caption.rs`

- [ ] **Step 1: Write the failing tests** (append to `mod tests`):

```rust
    #[tokio::test]
    async fn resolve_captioner_dir_uses_the_registry_role() {
        let db = empty_db().await;
        db.models()
            .insert(NewModel {
                name: "model.onnx".into(),
                format: "onnx".into(),
                file_path: "E:\\AI\\models\\vision\\wd-tagger\\model.onnx".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec!["vision_wd_tagger".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        let c = super::super::captioner::find(super::super::captioner::WD_TAGGER_ID).unwrap();
        let dir = resolve_captioner_dir(&db, c).await.unwrap();
        assert!(dir.to_string_lossy().replace('\\', "/").ends_with("vision/wd-tagger"));

        let florence = super::super::captioner::find(super::super::captioner::FLORENCE2_ID).unwrap();
        let err = resolve_captioner_dir(&db, florence).await.unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
    }

    #[test]
    fn rpc_method_and_engine_label_follow_the_captioner() {
        let wd = super::super::captioner::find(super::super::captioner::WD_TAGGER_ID).unwrap();
        assert_eq!(rpc_method_for(wd), "tag_frame");
        let fl = super::super::captioner::find(super::super::captioner::FLORENCE2_ID).unwrap();
        assert_eq!(rpc_method_for(fl), "caption_frame");
    }
```

- [ ] **Step 2: Run** `cargo test -p aiwm-core --lib capability::dataset::caption` → compile errors (`resolve_captioner_dir`, `rpc_method_for`).

- [ ] **Step 3: Implement** in `caption.rs`:

```rust
use super::captioner::{Captioner, FLORENCE2_ID};

pub async fn resolve_captioner_dir(db: &Database, c: &Captioner) -> Result<std::path::PathBuf> {
    resolve_model_dir(db, c.role, c.name.split(" (").next().unwrap_or(c.name)).await
}

/// Which sidecar JSON-RPC method serves this captioner.
pub fn rpc_method_for(c: &Captioner) -> &'static str {
    if c.id == FLORENCE2_ID {
        "caption_frame"
    } else {
        "tag_frame"
    }
}

/// One auto caption for a single frame from whichever captioner the run
/// picked. Returns `(caption, engine_label)`; the label is what lands in
/// `dataset_frames.caption_engine`.
pub async fn caption_with(
    vision: &VisionAdapter,
    c: &Captioner,
    model_dir: &Path,
    image_path: &Path,
) -> Result<(String, String)> {
    let client = vision.client().await?;
    let mut params = json!({
        "image_path": image_path.to_string_lossy(),
        "model_dir": model_dir.to_string_lossy(),
    });
    if c.id == FLORENCE2_ID {
        params["task_prompt"] = json!(FLORENCE2_TASK_PROMPT);
    }
    let result = client.call(rpc_method_for(c), params).await?;
    let engine = result
        .get("engine")
        .and_then(Value::as_str)
        .unwrap_or(c.id)
        .to_string();
    Ok((extract_caption(&result)?, engine))
}
```

Keep `caption_frame` (Florence-only) as is — `caption_with` is the new entry point `mod.rs` will use. The `label` passed to `resolve_model_dir` for Florence-2 stays `"Florence-2"` (the name is `"Florence-2 (prose)"`, split at `" ("`), so the existing error-message test keeps passing.

- [ ] **Step 4: Run** `cargo test -p aiwm-core --lib capability::dataset::caption` → all pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/capability/dataset/caption.rs
git commit -m "feat(dataset): captioner-aware caption_with dispatch

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: `extract.rs` — ffprobe duration, preview still, `ffprobe` resolution

**Files:**
- Modify: `core/src/capability/dataset/extract.rs`

- [ ] **Step 1: Write the failing tests** (append to its `mod tests`):

```rust
    #[test]
    fn ffprobe_args_ask_for_duration_only() {
        let args = ffprobe_duration_args(Path::new("E:\\v\\clip.mp4"));
        assert_eq!(
            args,
            vec![
                "-v", "error", "-show_entries", "format=duration", "-of",
                "default=noprint_wrappers=1:nokey=1", "E:\\v\\clip.mp4",
            ]
        );
    }

    #[test]
    fn parse_ffprobe_duration_accepts_seconds_and_rejects_garbage() {
        assert_eq!(parse_ffprobe_duration("12.480000\n"), Some(12.48));
        assert_eq!(parse_ffprobe_duration("N/A"), None);
        assert_eq!(parse_ffprobe_duration(""), None);
    }

    #[test]
    fn preview_still_args_seek_then_grab_one_frame() {
        let args = ffmpeg_preview_args(Path::new("E:\\v\\clip.mp4"), Path::new("E:\\o\\p.png"), 1.0);
        assert_eq!(
            args,
            vec!["-y", "-ss", "1", "-i", "E:\\v\\clip.mp4", "-frames:v", "1", "E:\\o\\p.png"]
        );
    }

    #[tokio::test]
    async fn probe_duration_reports_a_clear_error_when_ffprobe_cannot_spawn() {
        let err = probe_duration_secs(Path::new("C:\\definitely\\not\\ffprobe.exe"), Path::new("x.mp4"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("spawn ffprobe"), "{err}");
    }
```

- [ ] **Step 2: Run** `cargo test -p aiwm-core --lib capability::dataset::extract` → compile errors.

- [ ] **Step 3: Implement** (add to `extract.rs`):

```rust
/// `ffprobe` ships next to `ffmpeg` in every distribution AIWM cares about
/// (WinGet, the official builds) — same directory, same lookup.
pub fn resolve_ffprobe() -> Option<PathBuf> {
    let ffmpeg = resolve_ffmpeg()?;
    let dir = ffmpeg.parent()?;
    for name in ["ffprobe.exe", "ffprobe"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn ffprobe_duration_args(video: &Path) -> Vec<String> {
    vec![
        "-v".into(),
        "error".into(),
        "-show_entries".into(),
        "format=duration".into(),
        "-of".into(),
        "default=noprint_wrappers=1:nokey=1".into(),
        video.to_string_lossy().into_owned(),
    ]
}

pub fn parse_ffprobe_duration(stdout: &str) -> Option<f64> {
    stdout.trim().parse::<f64>().ok().filter(|d| d.is_finite() && *d >= 0.0)
}

/// `Ok(None)` when the file is not decodable / has no duration (the clip
/// gets rejected as `Unusable`), `Err` only when ffprobe itself cannot run.
pub async fn probe_duration_secs(ffprobe_bin: &Path, video: &Path) -> Result<Option<f64>> {
    let output = Command::new(ffprobe_bin)
        .args(ffprobe_duration_args(video))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| dataset_err(format!("spawn ffprobe: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(parse_ffprobe_duration(&String::from_utf8_lossy(&output.stdout)))
}

pub fn ffmpeg_preview_args(video: &Path, out_png: &Path, at_secs: f64) -> Vec<String> {
    vec![
        "-y".into(),
        "-ss".into(),
        format!("{at_secs}"),
        "-i".into(),
        video.to_string_lossy().into_owned(),
        "-frames:v".into(),
        "1".into(),
        out_png.to_string_lossy().into_owned(),
    ]
}

/// One still from `video` at `at_secs` — the clip-mode preview.
pub async fn extract_preview_still(
    ffmpeg_bin: &Path,
    video: &Path,
    out_png: &Path,
    at_secs: f64,
) -> Result<()> {
    if let Some(parent) = out_png.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| dataset_err(format!("create {}: {e}", parent.display())))?;
    }
    let output = Command::new(ffmpeg_bin)
        .args(ffmpeg_preview_args(video, out_png, at_secs))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| dataset_err(format!("spawn ffmpeg: {e}")))?;
    if !output.status.success() {
        return Err(dataset_err(format!(
            "ffmpeg preview failed on {}: {}",
            video.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}
```

- [ ] **Step 4: Run** the extract tests → pass. **Commit:**

```bash
git add core/src/capability/dataset/extract.rs
git commit -m "feat(dataset): ffprobe duration + preview still for clip mode

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Pipeline: request fields, dataset row, rejected frames, optional captioning, clip mode, composed export

**Files:**
- Modify: `core/src/capability/dataset/mod.rs`
- Modify: `core/src/orchestrator/engine.rs` (VRAM already reads the request; only the `run` call changes if its signature does — it does not)

This is the integration task; keep each step's tests green before the next.

- [ ] **Step 1: Request fields — failing tests** (append to `mod tests` in `mod.rs`):

```rust
    #[test]
    fn from_params_defaults_to_frames_mode_no_captioner_and_the_diversity_cap() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x" })).unwrap();
        assert_eq!(r.mode, crate::db::DatasetMode::Frames);
        assert_eq!(r.captioner, None);
        assert_eq!(r.max_frames_per_clip, filter::DEFAULT_MAX_FRAMES_PER_CLIP);
        assert_eq!(r.min_clip_secs, DEFAULT_MIN_CLIP_SECS);
        assert_eq!(r.vram_estimate_mb(), 0, "no captioner, no VRAM");
    }

    #[test]
    fn from_params_accepts_a_registry_captioner_and_rejects_unknown_ones() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "captioner": "florence2" }))
            .unwrap();
        assert_eq!(r.captioner.as_deref(), Some("florence2"));
        assert_eq!(r.vram_estimate_mb(), FLORENCE2_VRAM_FALLBACK_MB + QWEN_VL_VRAM_FALLBACK_MB);

        let tagger = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "captioner": "wd-eva02-tagger-v3", "escalate": true }))
            .unwrap();
        assert_eq!(tagger.vram_estimate_mb(), 0, "tagger runs on the CPU and cannot escalate");

        let err = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "captioner": "nope" })).unwrap_err();
        assert!(err.to_string().contains("unknown captioner"), "{err}");
    }

    #[test]
    fn from_params_parses_clip_mode_and_clamps_the_cap() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": "x", "mode": "clips", "max_frames_per_clip": 999_999, "min_clip_secs": -3.0
        }))
        .unwrap();
        assert_eq!(r.mode, crate::db::DatasetMode::Clips);
        assert_eq!(r.max_frames_per_clip, MAX_FRAMES_PER_CLIP_CEILING);
        assert_eq!(r.min_clip_secs, 0.0);
        let err = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "mode": "stills" })).unwrap_err();
        assert!(err.to_string().contains("mode"), "{err}");
    }
```

- [ ] **Step 2: Implement the request changes.** Add fields and constants:

```rust
use crate::db::DatasetMode;

pub const DEFAULT_MIN_CLIP_SECS: f64 = 2.0;
const MAX_FRAMES_PER_CLIP_CEILING: usize = 10_000;

pub struct DatasetPrepRequest {
    pub root: PathBuf,
    pub mode: DatasetMode,
    /// A `captioner::CAPTIONERS` id, or `None` = extract/filter/curate only.
    pub captioner: Option<String>,
    pub sample_fps: f64,
    pub blur_threshold: f64,
    pub phash_max_distance: u32,
    pub max_frames_per_clip: usize,
    pub min_clip_secs: f64,
    pub escalate: bool,
    pub escalate_every_nth: u32,
    pub context_offset: usize,
}
```

In `from_params`, after `root`:

```rust
        let mode = match params.get("mode").and_then(Value::as_str) {
            None => DatasetMode::Frames,
            Some(s) => DatasetMode::parse(s)
                .ok_or_else(|| dataset_err(format!("unknown dataset mode {s:?} (expected frames or clips)")))?,
        };
        let captioner = match params.get("captioner").and_then(Value::as_str).map(str::trim) {
            None | Some("") => None,
            Some(id) => {
                captioner::find(id).ok_or_else(|| dataset_err(format!("unknown captioner {id:?}")))?;
                Some(id.to_string())
            }
        };
        let max_frames_per_clip = params
            .get("max_frames_per_clip")
            .and_then(Value::as_u64)
            .map_or(filter::DEFAULT_MAX_FRAMES_PER_CLIP, |v| {
                usize::try_from(v).unwrap_or(MAX_FRAMES_PER_CLIP_CEILING).min(MAX_FRAMES_PER_CLIP_CEILING)
            });
        let min_clip_secs = params
            .get("min_clip_secs")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_MIN_CLIP_SECS, |v| v.max(0.0));
```

`apply_to` writes `mode` (`self.mode.as_str()`), `captioner` (string or `Value::Null`), `max_frames_per_clip`, `min_clip_secs`. `vram_estimate_mb` becomes:

```rust
    pub fn vram_estimate_mb(&self) -> u64 {
        let Some(c) = self.captioner.as_deref().and_then(captioner::find) else {
            return 0;
        };
        c.vram_mb + if self.escalate && c.supports_escalation { caption::QWEN_VL_VRAM_FALLBACK_MB } else { 0 }
    }
```

Update the existing `vram_estimate_adds_qwen_only_when_escalating` test to pass `"captioner": "florence2"`, and `run_reports_a_clear_error_when_florence2_is_not_imported` to pass `"captioner": "florence2"` too (without a captioner the run no longer needs Florence-2 — that is the point). Run the `mod` tests → pass.

- [ ] **Step 3: `run` — failing integration-style test** (append; uses a temp root with one real PNG so no ffmpeg is needed):

```rust
    #[tokio::test]
    async fn run_without_a_captioner_creates_a_dataset_and_keeps_and_rejects_frames_with_reasons() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let job = db.jobs().insert(crate::db::NewJob::new("dataset_prep")).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let tag_dir = root.path().join("MyStyle");
        std::fs::create_dir_all(&tag_dir).unwrap();
        // A sharp image (kept) and a black one (rejected as dead).
        let sharp = image::DynamicImage::ImageLuma8({
            let mut b = image::GrayImage::new(32, 32);
            for y in 0..32 { for x in 0..32 { b.put_pixel(x, y, image::Luma([if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 }])); } }
            b
        });
        sharp.save(tag_dir.join("a.png")).unwrap();
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(32, 32, image::Luma([2]))).save(tag_dir.join("b.png")).unwrap();

        let req = DatasetPrepRequest::from_params(&serde_json::json!({ "root": root.path().to_string_lossy() })).unwrap();
        let work = tempfile::tempdir().unwrap();
        let (_tx, rx) = watch::channel(false);
        let outcome = run(&db, &vision, work.path(), &job.id, req, rx).await.unwrap();
        let DatasetPrepOutcome::Done(done) = outcome else { panic!("expected Done") };
        assert_eq!(done.frame_count, 1, "only the sharp frame is kept");

        let datasets = db.datasets().list().await.unwrap();
        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0].name, root.path().file_name().unwrap().to_string_lossy());
        assert_eq!(datasets[0].prep_job_id.as_deref(), Some(job.id.as_str()));

        let frames = db.dataset_frames().list_for_dataset(&datasets[0].id).await.unwrap();
        assert_eq!(frames.len(), 2, "rejected frames are stored too");
        let reasons: Vec<&str> = frames.iter().map(|f| f.rejection_reason.as_str()).collect();
        assert!(reasons.contains(&""), "{reasons:?}");
        assert!(reasons.contains(&"black"), "{reasons:?}");
        assert!(frames.iter().all(|f| f.caption.is_empty()), "no captioner -> no captions");
    }
```

- [ ] **Step 4: Rewrite `run` and `filter_groups`.** Replace the body of `run` with this shape (full function):

```rust
pub async fn run(
    db: &Database,
    vision: &VisionAdapter,
    work_dir: &Path,
    job_id: &str,
    req: DatasetPrepRequest,
    cancel: watch::Receiver<bool>,
) -> Result<DatasetPrepOutcome> {
    // Resolve the captioner (and its escalation partner) *before* touching
    // the disk, so a missing model fails fast — but only when one was asked
    // for. Extraction/filtering/curation never need a model.
    let captioner = req.captioner.as_deref().and_then(captioner::find);
    let captioner_dir = match captioner {
        Some(c) => Some(caption::resolve_captioner_dir(db, c).await?),
        None => None,
    };
    let escalate = req.escalate && captioner.is_some_and(|c| c.supports_escalation);
    let qwen_dir = if escalate {
        match caption::resolve_qwen_vl_dir(db).await {
            Ok(dir) => Some(dir),
            Err(e) => {
                db.jobs()
                    .append_event(job_id, EventLevel::Warn, &format!("temporal-context escalation disabled \u{2014} {e}"))
                    .await?;
                None
            }
        }
    } else {
        None
    };
    let escalate = escalate && qwen_dir.is_some();

    let items = ingest::walk_dataset_root(&req.root)?;
    if items.is_empty() {
        return Err(dataset_err(format!(
            "no videos or images found under {} (expected tag subfolders containing .mp4/.png/.jpg/.jpeg/.webp files)",
            req.root.display()
        )));
    }
    let dataset = db
        .datasets()
        .create(crate::db::NewDataset {
            name: req.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "dataset".into()),
            mode: req.mode,
            source_root: req.root.to_string_lossy().into_owned(),
            prep_job_id: Some(job_id.to_string()),
        })
        .await?;
    let tag_count: std::collections::BTreeSet<&String> = items.iter().map(|i| &i.tag).collect();
    db.jobs()
        .append_event(job_id, EventLevel::Info, &format!("found {} tag folder(s), {} source file(s)", tag_count.len(), items.len()))
        .await?;

    if req.mode == DatasetMode::Clips {
        return run_clip_mode(db, work_dir, job_id, &dataset.id, &items, &req, cancel).await;
    }

    let raw_dir = work_dir.join(job_id).join("raw");
    let mut groups: Vec<CandidateGroup> = Vec::new();
    for item in &items {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let frames = match item.kind {
            ingest::IngestKind::Video => {
                let ffmpeg = extract::resolve_ffmpeg().ok_or_else(|| dataset_err("ffmpeg was not found on PATH \u{2014} install it to extract frames from video"))?;
                let out_dir = raw_dir.join(&item.tag).join(video_stem(&item.path));
                extract::extract_frames(&ffmpeg, &item.path, &out_dir, req.sample_fps)
                    .await?
                    .into_iter()
                    .map(|f| FrameCandidate { path: f.path, source: item.path.clone(), tag: item.tag.clone(), timestamp_secs: Some(f.timestamp_secs) })
                    .collect()
            }
            ingest::IngestKind::Image => vec![FrameCandidate { path: item.path.clone(), source: item.path.clone(), tag: item.tag.clone(), timestamp_secs: None }],
        };
        db.jobs()
            .append_event(job_id, EventLevel::Info, &format!("{} still(s) from {}", frames.len(), item.path.display()))
            .await?;
        groups.push(CandidateGroup { source: item.path.clone(), frames });
    }

    let Some((judged_groups, total_extracted, total_kept)) = filter_groups(&cancel, groups, &req)? else {
        return Ok(DatasetPrepOutcome::Cancelled);
    };
    db.jobs()
        .append_event(job_id, EventLevel::Info, &format!("kept {total_kept} of {total_extracted} frame(s) after filtering"))
        .await?;
    if total_kept == 0 {
        return Err(dataset_err("every extracted frame was filtered out \u{2014} try a lower blur threshold, a higher sample rate, or a larger per-clip cap"));
    }

    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut frame_count = 0usize;
    for group in &judged_groups {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let mut kept_records: Vec<DatasetFrame> = Vec::new();
        for (cand, reason) in &group.frames {
            let row = db
                .dataset_frames()
                .insert(NewDatasetFrame {
                    job_id: job_id.to_string(),
                    dataset_id: Some(dataset.id.clone()),
                    tag: cand.tag.clone(),
                    source_path: cand.source.to_string_lossy().into_owned(),
                    frame_path: cand.path.to_string_lossy().into_owned(),
                    timestamp_secs: cand.timestamp_secs,
                    rejection_reason: reason.map(filter::RejectionReason::as_str).unwrap_or("").to_string(),
                    duration_secs: None,
                })
                .await?;
            if reason.is_none() {
                *tag_counts.entry(cand.tag.clone()).or_insert(0) += 1;
                frame_count += 1;
                kept_records.push(row);
            }
        }

        if let (Some(c), Some(dir)) = (captioner, captioner_dir.as_deref()) {
            caption_group(db, vision, c, dir, qwen_dir.as_deref(), escalate, &req, &kept_records, &cancel).await?;
            if cancelled(&cancel).await {
                return Ok(DatasetPrepOutcome::Cancelled);
            }
            db.jobs()
                .append_event(job_id, EventLevel::Info, &format!("captioned {} frame(s) from {}", kept_records.len(), group.source.display()))
                .await?;
        }
    }

    Ok(DatasetPrepOutcome::Done(DatasetPrepDone { frame_count, tag_counts }))
}
```

`CandidateGroup` for the judged stage becomes a distinct type:

```rust
/// A group after filtering: every candidate, each with its verdict.
struct JudgedGroup {
    source: PathBuf,
    frames: Vec<(FrameCandidate, Option<filter::RejectionReason>)>,
}
```

and `filter_groups` implements filter level C in spec order (dead → transition → blur/dup → cap), returning `Vec<JudgedGroup>`:

```rust
fn filter_groups(
    cancel: &watch::Receiver<bool>,
    groups: Vec<CandidateGroup>,
    req: &DatasetPrepRequest,
) -> Result<Option<(Vec<JudgedGroup>, usize, usize)>> {
    use filter::RejectionReason as R;
    let mut judged_groups = Vec::new();
    let mut total_extracted = 0usize;
    let mut total_kept = 0usize;
    for group in groups {
        if *cancel.borrow() {
            return Ok(None);
        }
        total_extracted += group.frames.len();
        let n = group.frames.len();

        // Pass 1: per-frame facts.
        let mut dead = Vec::with_capacity(n);
        let mut blurry = Vec::with_capacity(n);
        let mut hashes = Vec::with_capacity(n);
        for cand in &group.frames {
            dead.push(filter::is_dead_frame(&cand.path)?);
            blurry.push(filter::is_blurry(&cand.path, req.blur_threshold)?);
            hashes.push(filter::phash_of(&cand.path)?);
        }

        // Pass 2: verdicts in spec order; a frame keeps its *first* reason.
        let mut verdict: Vec<Option<R>> = vec![None; n];
        for i in 0..n {
            if dead[i] {
                verdict[i] = Some(R::Black);
            }
        }
        for i in 0..n {
            if verdict[i].is_none()
                && filter::is_transition(
                    blurry[i],
                    (i > 0).then(|| &hashes[i - 1]),
                    &hashes[i],
                    hashes.get(i + 1),
                    filter::DEFAULT_TRANSITION_MIN_DISTANCE,
                )
            {
                verdict[i] = Some(R::Transition);
            }
        }
        let mut last_kept: Option<&image_hasher::ImageHash> = None;
        for i in 0..n {
            if verdict[i].is_some() {
                continue;
            }
            if blurry[i] {
                verdict[i] = Some(R::Blur);
                continue;
            }
            if let Some(prev) = last_kept {
                if prev.dist(&hashes[i]) <= req.phash_max_distance {
                    verdict[i] = Some(R::Duplicate);
                    continue;
                }
            }
            last_kept = Some(&hashes[i]);
        }
        // Pass 3: diversity cap over what survived.
        let survivors: Vec<usize> = (0..n).filter(|i| verdict[*i].is_none()).collect();
        let survivor_hashes: Vec<image_hasher::ImageHash> = survivors.iter().map(|i| hashes[*i].clone()).collect();
        let chosen = filter::select_diverse(&survivor_hashes, req.max_frames_per_clip);
        for (k, i) in survivors.iter().enumerate() {
            if !chosen.contains(&k) {
                verdict[*i] = Some(R::Cap);
            }
        }

        total_kept += verdict.iter().filter(|v| v.is_none()).count();
        judged_groups.push(JudgedGroup {
            source: group.source,
            frames: group.frames.into_iter().zip(verdict).collect(),
        });
    }
    Ok(Some((judged_groups, total_extracted, total_kept)))
}
```

`caption_group` changes signature to take `c: &Captioner, model_dir: &Path` instead of `florence_dir`, and its first line becomes `let (base_caption, mut engine) = caption::caption_with(vision, c, model_dir, frame_path).await?;` with `engine` a `String` (assign `"qwen2.5-vl".to_string()` on escalation). Escalation only when `c.supports_escalation`.

- [ ] **Step 5: Clip mode** — add:

```rust
async fn run_clip_mode(
    db: &Database,
    work_dir: &Path,
    job_id: &str,
    dataset_id: &str,
    items: &[ingest::IngestItem],
    req: &DatasetPrepRequest,
    cancel: watch::Receiver<bool>,
) -> Result<DatasetPrepOutcome> {
    let ffmpeg = extract::resolve_ffmpeg().ok_or_else(|| dataset_err("ffmpeg was not found on PATH"))?;
    let ffprobe = extract::resolve_ffprobe().ok_or_else(|| dataset_err("ffprobe was not found next to ffmpeg"))?;
    let preview_dir = work_dir.join(job_id).join("previews");
    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut kept = 0usize;
    for item in items.iter().filter(|i| i.kind == ingest::IngestKind::Video) {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let duration = extract::probe_duration_secs(&ffprobe, &item.path).await?;
        let usable = duration.is_some_and(|d| d >= req.min_clip_secs);
        let preview = preview_dir.join(&item.tag).join(format!("{}.png", video_stem(&item.path)));
        if usable {
            let at = duration.unwrap_or(0.0).min(1.0);
            extract::extract_preview_still(&ffmpeg, &item.path, &preview, at).await?;
        }
        db.dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job_id.to_string(),
                dataset_id: Some(dataset_id.to_string()),
                tag: item.tag.clone(),
                source_path: item.path.to_string_lossy().into_owned(),
                frame_path: if usable { preview.to_string_lossy().into_owned() } else { String::new() },
                timestamp_secs: None,
                rejection_reason: if usable { String::new() } else { filter::RejectionReason::Unusable.as_str().into() },
                duration_secs: duration,
            })
            .await?;
        if usable {
            *tag_counts.entry(item.tag.clone()).or_insert(0) += 1;
            kept += 1;
        }
    }
    if kept == 0 {
        return Err(dataset_err("no usable clips \u{2014} every video was undecodable or shorter than the minimum length"));
    }
    db.jobs()
        .append_event(job_id, EventLevel::Info, &format!("{kept} usable clip(s) across {} tag(s)", tag_counts.len()))
        .await?;
    Ok(DatasetPrepOutcome::Done(DatasetPrepDone { frame_count: kept, tag_counts }))
}
```

(`ingest::IngestKind` needs `#[derive(PartialEq)]` if it lacks it; `ingest::IngestItem` is already `pub`.)

- [ ] **Step 6: Composed export.** Replace `export_dataset` with a dataset-keyed version plus a job-keyed shim:

```rust
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub dataset_id: String,
    pub dest_dir: PathBuf,
    pub caption_order: compose::CaptionOrder,
}

/// Write every kept, non-excluded item to `dest_dir`. Frames mode: `NNNN.<ext>`
/// + `NNNN.txt` with the caption composed from the dataset trigger, the
/// frame's concepts and its auto/hand caption (spec 3A/3D). Clips mode: the
/// clip file (trimmed with ffmpeg when a range is set) + `NNNN.txt`.
pub async fn export_dataset(db: &Database, req: &ExportRequest) -> Result<ExportSummary> {
    let dataset = db
        .datasets()
        .get(&req.dataset_id)
        .await?
        .ok_or_else(|| dataset_err(format!("no such dataset {}", req.dataset_id)))?;
    let frames = db.dataset_frames().list_for_dataset(&dataset.id).await?;
    let kept: Vec<DatasetFrame> = frames.into_iter().filter(|f| !f.excluded && f.rejection_reason.is_empty()).collect();
    if kept.is_empty() {
        return Err(dataset_err("nothing to export \u{2014} every item is excluded or rejected"));
    }
    let concepts = db.concepts().list_for_dataset(&dataset.id).await?;
    let concept_map = db.concepts().map_for_dataset(&dataset.id).await?;
    let by_id: std::collections::HashMap<&str, &crate::db::DatasetConcept> = concepts.iter().map(|c| (c.id.as_str(), c)).collect();

    tokio::fs::create_dir_all(&req.dest_dir)
        .await
        .map_err(|e| dataset_err(format!("create {}: {e}", req.dest_dir.display())))?;

    let ffmpeg = if dataset.mode == DatasetMode::Clips { extract::resolve_ffmpeg() } else { None };
    for (i, frame) in kept.iter().enumerate() {
        let n = i + 1;
        let parts: Vec<compose::ConceptPart<'_>> = concept_map
            .get(&frame.id)
            .map(|ids| ids.iter().filter_map(|id| by_id.get(id.as_str())).map(|c| compose::ConceptPart { token: &c.token, description: &c.description }).collect())
            .unwrap_or_default();
        let style = captioner::find(&frame.caption_engine).map_or(compose::CaptionStyle::Prose, |c| c.style);
        let caption = compose::compose_caption(req.caption_order, &dataset.trigger_word, &parts, &frame.caption, style);

        let src = if dataset.mode == DatasetMode::Clips { &frame.source_path } else { &frame.frame_path };
        let ext = Path::new(src).extension().and_then(|e| e.to_str()).unwrap_or(if dataset.mode == DatasetMode::Clips { "mp4" } else { "png" });
        let media_dest = req.dest_dir.join(format!("{n:04}.{ext}"));
        match (dataset.mode, frame.clip_start_secs, frame.clip_end_secs, ffmpeg.as_deref()) {
            (DatasetMode::Clips, start, end, Some(ffmpeg)) if start.is_some() || end.is_some() => {
                extract::trim_clip(ffmpeg, Path::new(src), &media_dest, start, end).await?;
            }
            _ => {
                tokio::fs::copy(src, &media_dest)
                    .await
                    .map_err(|e| dataset_err(format!("copy {src} \u{2192} {}: {e}", media_dest.display())))?;
            }
        }
        let caption_dest = req.dest_dir.join(format!("{n:04}.txt"));
        tokio::fs::write(&caption_dest, caption.as_bytes())
            .await
            .map_err(|e| dataset_err(format!("write {}: {e}", caption_dest.display())))?;
    }
    db.datasets().set_export_dir(&dataset.id, &req.dest_dir.to_string_lossy()).await?;
    Ok(ExportSummary { exported: kept.len(), dest_dir: req.dest_dir.clone() })
}

/// Job-keyed shim for the existing `POST /jobs/{id}/dataset-export` route:
/// resolves the dataset the job produced, prose-first.
pub async fn export_dataset_for_job(db: &Database, job_id: &str, dest_dir: &Path) -> Result<ExportSummary> {
    let dataset = db
        .datasets()
        .list()
        .await?
        .into_iter()
        .find(|d| d.prep_job_id.as_deref() == Some(job_id))
        .ok_or_else(|| dataset_err("this job produced no dataset"))?;
    export_dataset(db, &ExportRequest { dataset_id: dataset.id, dest_dir: dest_dir.to_path_buf(), caption_order: compose::CaptionOrder::ProseFirst }).await
}
```

Add `extract::trim_clip` (and a `ffmpeg_trim_args` builder with a unit test asserting `["-y", "-ss", "<start>", "-to", "<end>", "-i", src, "-c", "copy", dest]` with `-ss`/`-to` omitted when `None`) in `extract.rs`. Rewrite the two existing export tests to create a dataset (trigger `"ghibli_xy"`), a concept `kenji_xy` assigned to frame 0, and assert `0001.txt == "caption 0, ghibli_xy, kenji_xy"` (prose-first) and that `CaptionOrder::TagsFirst` yields `"ghibli_xy, kenji_xy, caption 0"`; also assert a rejected frame (`rejection_reason: "blur"`) is not exported.

- [ ] **Step 7: Engine.** `core/src/orchestrator/engine.rs` line ~1230: the call `dataset::run(&self.db, &vision, &work_dir, &job.id, req, cancel)` is unchanged. The VRAM branch (line ~373) already uses `req.vram_estimate_mb()`, now `0` without a captioner. Add one engine test next to the existing dataset_prep target test: a `dataset_prep` job without `captioner` resolves a `Target` with `vram_mb == 0`.

- [ ] **Step 8: Gates and commit**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all green.

```bash
git add core/src/capability/dataset core/src/orchestrator/engine.rs
git commit -m "feat(dataset): datasets as objects, rejected frames kept with reasons, optional captioner, clip mode, composed export

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: API — DTOs, handlers, HTTP routes, Tauri commands, `ipc.ts`, hooks, dev-mock

**Files:**
- Modify: `core/src/api/dto.rs`, `core/src/api/handlers.rs`, `core/src/api/http.rs`, `src-tauri/src/lib.rs`, `ui/src/lib/ipc.ts`, `ui/src/lib/hooks.ts`, `ui/src/lib/dev-mock.ts`

Every route below follows the existing `list_dataset_frames`/`update_dataset_frame`/`export_dataset` trio exactly (handler in `handlers.rs` → axum fn in `http.rs` → `#[tauri::command]` in `lib.rs` → `invoke` wrapper in `ipc.ts` → case in `dev-mock.ts`).

- [ ] **Step 1: DTOs** (`dto.rs`):

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDatasetDto {
    #[serde(default)]
    pub trigger_word: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConceptBodyDto {
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConceptFramesDto {
    pub frame_ids: Vec<String>,
}

/// Extends the frame edit: `restore: true` clears a rejection.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDatasetFrameDto {
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub excluded: Option<bool>,
    #[serde(default)]
    pub restore: Option<bool>,
    #[serde(default)]
    pub clip_start_secs: Option<Option<f64>>,
    #[serde(default)]
    pub clip_end_secs: Option<Option<f64>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportDatasetDto {
    pub dest_dir: String,
    #[serde(default = "default_caption_order")]
    pub caption_order: crate::capability::dataset::CaptionOrder,
}

fn default_caption_order() -> crate::capability::dataset::CaptionOrder {
    crate::capability::dataset::CaptionOrder::ProseFirst
}

/// `GET /datasets/{id}/concepts` — concepts with their frame counts and a
/// token warning when the token looks like an ordinary word.
#[derive(Debug, Clone, Serialize)]
pub struct ConceptSummaryDto {
    #[serde(flatten)]
    pub concept: crate::db::DatasetConcept,
    pub frame_count: usize,
    pub token_warning: Option<String>,
}
```

- [ ] **Step 2: Handlers** (`handlers.rs`, in the dataset section):

```rust
pub async fn list_captioners(app: &App) -> Result<Vec<crate::capability::dataset::CaptionerStatus>> {
    crate::capability::dataset::captioner::statuses(&app.db).await
}

pub async fn list_datasets(app: &App) -> Result<Vec<crate::db::Dataset>> {
    app.db.datasets().list().await
}

pub async fn get_dataset(app: &App, id: &str) -> Result<Option<crate::db::Dataset>> {
    app.db.datasets().get(id).await
}

pub async fn update_dataset(app: &App, id: &str, body: UpdateDatasetDto) -> Result<crate::db::Dataset> {
    if let Some(t) = body.trigger_word {
        app.db.datasets().set_trigger_word(id, &t).await?;
    }
    app.db.datasets().get(id).await?.ok_or_else(|| CoreError::Config(format!("no such dataset {id}")))
}

pub async fn delete_dataset(app: &App, id: &str) -> Result<()> {
    app.db.datasets().delete(id).await
}

pub async fn list_dataset_frames_for_dataset(app: &App, dataset_id: &str) -> Result<Vec<crate::db::DatasetFrame>> {
    app.db.dataset_frames().list_for_dataset(dataset_id).await
}

pub async fn list_concepts(app: &App, dataset_id: &str) -> Result<Vec<ConceptSummaryDto>> {
    let concepts = app.db.concepts().list_for_dataset(dataset_id).await?;
    let counts = app.db.concepts().counts_for_dataset(dataset_id).await?;
    Ok(concepts
        .into_iter()
        .map(|c| ConceptSummaryDto {
            frame_count: counts.get(&c.id).copied().unwrap_or(0),
            token_warning: crate::capability::dataset::token_warning(&c.token),
            concept: c,
        })
        .collect())
}

pub async fn create_concept(app: &App, dataset_id: &str, body: ConceptBodyDto) -> Result<crate::db::DatasetConcept> {
    if body.name.trim().is_empty() || body.token.trim().is_empty() {
        return Err(CoreError::Config("concept name and token must not be empty".into()));
    }
    app.db
        .concepts()
        .create(crate::db::NewConcept { dataset_id: dataset_id.to_string(), name: body.name, token: body.token, description: body.description })
        .await
}

pub async fn update_concept(app: &App, id: &str, body: ConceptBodyDto) -> Result<()> {
    app.db.concepts().update(id, &body.name, &body.token, &body.description).await
}

pub async fn delete_concept(app: &App, id: &str) -> Result<()> {
    app.db.concepts().delete(id).await
}

pub async fn assign_concept(app: &App, concept_id: &str, body: ConceptFramesDto) -> Result<()> {
    app.db.concepts().assign(concept_id, &body.frame_ids).await
}

pub async fn unassign_concept(app: &App, concept_id: &str, body: ConceptFramesDto) -> Result<()> {
    app.db.concepts().unassign(concept_id, &body.frame_ids).await
}

pub async fn frame_concept_map(app: &App, dataset_id: &str) -> Result<std::collections::HashMap<String, Vec<String>>> {
    app.db.concepts().map_for_dataset(dataset_id).await
}

pub async fn export_dataset_by_id(app: &App, dataset_id: &str, body: ExportDatasetDto) -> Result<crate::capability::dataset::ExportSummary> {
    crate::capability::dataset::export_dataset(
        &app.db,
        &crate::capability::dataset::ExportRequest { dataset_id: dataset_id.to_string(), dest_dir: PathBuf::from(body.dest_dir), caption_order: body.caption_order },
    )
    .await
}
```

Extend the existing `update_dataset_frame` handler: `if body.restore == Some(true) { set_rejection_reason(frame_id, "") }`; `if body.clip_start_secs.is_some() || body.clip_end_secs.is_some() { set_clip_range(frame_id, body.clip_start_secs.flatten(), body.clip_end_secs.flatten()) }`. Point the existing job-keyed `export_dataset` handler at `export_dataset_for_job`.

- [ ] **Step 3: Routes** (`http.rs` router), after the existing dataset routes:

```rust
        .route("/captioners", get(list_captioners))
        .route("/datasets", get(list_datasets))
        .route("/datasets/{id}", get(get_dataset).put(update_dataset).delete(delete_dataset))
        .route("/datasets/{id}/frames", get(list_dataset_frames_for_dataset))
        .route("/datasets/{id}/frames/{frame_id}/image", get(dataset_frame_image))
        .route("/datasets/{id}/frame-concepts", get(frame_concept_map))
        .route("/datasets/{id}/concepts", get(list_concepts).post(create_concept))
        .route("/datasets/{id}/export", post(export_dataset_by_id))
        .route("/concepts/{id}", put(update_concept).delete(delete_concept))
        .route("/concepts/{id}/frames", post(assign_concept).delete(unassign_concept))
```

with thin axum fns in the same shape as `list_dataset_frames`/`update_dataset_frame` (`State(app)`, `Path(id)`, `Json(body)`; `POST` create → `StatusCode::CREATED`; `get_dataset` → 404 JSON when `None`, like `job_detail`). The `frame_concepts` and `PUT /jobs/{id}/dataset-frames/{frame_id}` existing route keeps working.

- [ ] **Step 4: Tauri commands** (`src-tauri/src/lib.rs`): `list_captioners`, `list_datasets`, `get_dataset`, `update_dataset(id, body)`, `delete_dataset(id)`, `list_dataset_frames_for_dataset(dataset_id)`, `frame_concept_map(dataset_id)`, `list_concepts(dataset_id)`, `create_concept(dataset_id, body)`, `update_concept(id, body)`, `delete_concept(id)`, `assign_concept(concept_id, body)`, `unassign_concept(concept_id, body)`, `export_dataset_by_id(dataset_id, body)` — each `to_ipc(handlers::...)`, each added to `generate_handler!`.

- [ ] **Step 5: `ipc.ts`** — add types and bindings:

```ts
export type DatasetMode = "frames" | "clips";
export type CaptionOrder = "tags_first" | "prose_first";
export type CaptionStyle = "prose" | "tags";

export interface Dataset {
  id: string;
  name: string;
  mode: DatasetMode;
  source_root: string;
  trigger_word: string;
  prep_job_id: string | null;
  export_dir: string | null;
  created_at: string;
}

export interface Captioner {
  id: string;
  name: string;
  style: CaptionStyle;
  role: string;
  vram_mb: number;
  license: string;
  supports_escalation: boolean;
  installed: boolean;
}

export interface DatasetConcept {
  id: string;
  dataset_id: string;
  name: string;
  token: string;
  description: string;
  created_at: string;
  frame_count: number;
  token_warning: string | null;
}

export const listCaptioners = () => invoke<Captioner[]>("list_captioners");
export const listDatasets = () => invoke<Dataset[]>("list_datasets");
export const getDataset = (id: string) => invoke<Dataset | null>("get_dataset", { id });
export const updateDataset = (id: string, body: { trigger_word?: string }) =>
  invoke<Dataset>("update_dataset", { id, body });
export const deleteDataset = (id: string) => invoke<void>("delete_dataset", { id });
export const listDatasetFramesForDataset = (datasetId: string) =>
  invoke<DatasetFrame[]>("list_dataset_frames_for_dataset", { datasetId });
export const frameConceptMap = (datasetId: string) =>
  invoke<Record<string, string[]>>("frame_concept_map", { datasetId });
export const listConcepts = (datasetId: string) => invoke<DatasetConcept[]>("list_concepts", { datasetId });
export const createConcept = (datasetId: string, body: { name: string; token: string; description?: string }) =>
  invoke<DatasetConcept>("create_concept", { datasetId, body });
export const updateConcept = (id: string, body: { name: string; token: string; description?: string }) =>
  invoke<void>("update_concept", { id, body });
export const deleteConcept = (id: string) => invoke<void>("delete_concept", { id });
export const assignConcept = (conceptId: string, frameIds: string[]) =>
  invoke<void>("assign_concept", { conceptId, body: { frame_ids: frameIds } });
export const unassignConcept = (conceptId: string, frameIds: string[]) =>
  invoke<void>("unassign_concept", { conceptId, body: { frame_ids: frameIds } });
/** Dataset-keyed still image — replaces `datasetFrameImageUrl(port, frame.job_id, id)`
 *  now that `frame.job_id` can be null (a frame outlives its prep job). The
 *  existing job-keyed route stays for old callers. */
export const datasetFrameImageUrlByDataset = (coreApiPort: number, datasetId: string, frameId: string) =>
  `http://127.0.0.1:${coreApiPort}/datasets/${datasetId}/frames/${frameId}/image`;

export const exportDatasetById = (datasetId: string, destDir: string, captionOrder: CaptionOrder) =>
  invoke<ExportDatasetSummary>("export_dataset_by_id", { datasetId, body: { dest_dir: destDir, caption_order: captionOrder } });
```

Extend `DatasetFrame` with `dataset_id: string | null; rejection_reason: string; duration_secs: number | null; clip_start_secs: number | null; clip_end_secs: number | null;`, `DatasetPrepParams` with `mode?: DatasetMode; captioner?: string | null; max_frames_per_clip?: number; min_clip_secs?: number;`, and `updateDatasetFrame`'s body type with `restore?: boolean; clip_start_secs?: number | null; clip_end_secs?: number | null;`.

- [ ] **Step 6: `hooks.ts`** — `useCaptioners()` (polled 5 s), `useDatasets()` (polled 3 s), `useDatasetFramesForDataset(datasetId)` (3 s), `useConcepts(datasetId)` (3 s), `useFrameConceptMap(datasetId)` (3 s) — each via the existing `usePolled` helper, same shape as `useDatasetFrames`.

- [ ] **Step 7: `dev-mock.ts`** — add `DATASETS`, `CONCEPTS`, `FRAME_CONCEPTS` in-memory arrays; `progressDatasetJobs` creates a `datasets` row per running job (mode from params, name from the root's last path segment) and marks every 7th mock frame `rejection_reason: "blur"` and every 11th `"cap"`; cases for each new command (`list_captioners` returns Florence-2 `installed: false` and the WD tagger `installed: true` so both UI states are visible; `export_dataset_by_id` returns `{ exported: n, dest_dir }`; `update_dataset_frame` honours `restore`). Run `pnpm typecheck && pnpm lint` in `ui/`.

- [ ] **Step 8: HTTP integration test** — in `core/tests/`, add `dataset_api.rs` (mirror `core/tests/civitai_registry.rs`'s harness style if it boots a real router; otherwise the existing HTTP integration test pattern the Story Studio flow used): create a dataset row directly via `db.datasets().create(...)`, then over HTTP: `POST /datasets/{id}/concepts` → 201; `POST /concepts/{id}/frames` with two frame ids; `GET /datasets/{id}/frame-concepts` shows both; `GET /datasets/{id}/concepts` shows `frame_count: 2` and a `token_warning` for a concept whose token is `"anime"`; `DELETE /concepts/{id}` → 204 and the map is empty.

- [ ] **Step 9: Gates + commit**

`cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` and `pnpm typecheck && pnpm lint` (ui) all green.

```bash
git add core/src/api src-tauri/src/lib.rs ui/src/lib core/tests/dataset_api.rs
git commit -m "feat(api): datasets, concepts, captioners, and export-with-order over HTTP + Tauri

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Dataset tab — captioner choice, mode, rejection chips, dataset picker, concepts panel

**Files:**
- Modify: `ui/src/features/dataset/Dataset.tsx`, `ui/src/features/dataset/dataset.css`
- Create: `ui/src/features/dataset/ConceptsPanel.tsx`

- [ ] **Step 1: Form changes** in `Dataset.tsx`:
  - Replace the escalate checkbox block with a **captioning section**:

```tsx
const { data: captioners } = useCaptioners();
const installed = (captioners ?? []).filter((c) => c.installed);
const [captionerId, setCaptionerId] = useState<string | null>(null);
useEffect(() => {
  // Default: the first installed captioner (spec 3: off when none is installed).
  if (captionerId === null && installed.length > 0) setCaptionerId(installed[0].id);
  // eslint-disable-next-line react-hooks/exhaustive-deps
}, [installed.length]);
const captionOn = captionerId !== null;
const chosen = (captioners ?? []).find((c) => c.id === captionerId) ?? null;
```

```tsx
<fieldset className="datasetform__captioning">
  <legend>Auto-caption</legend>
  <label className="datasetform__check">
    <input
      type="checkbox"
      checked={captionOn}
      disabled={installed.length === 0}
      onChange={(e) => setCaptionerId(e.target.checked ? (installed[0]?.id ?? null) : null)}
    />
    <span>
      Describe every kept frame automatically.{" "}
      <em>Recommended for style LoRAs: what is described stays controllable, what is not becomes part of the style.</em>
    </span>
  </label>
  {installed.length === 0 && (
    <p className="datasetform__hint">
      No captioner installed — import Florence-2 or the WD tagger on the Models tab. Without one, everything recurring
      in your frames flows into the trigger word.
    </p>
  )}
  {captionOn && (
    <label className="datasetform__field datasetform__field--inline">
      <span>Describe with</span>
      <select value={captionerId ?? ""} onChange={(e) => setCaptionerId(e.target.value)}>
        {installed.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name} {c.style === "tags" ? "· tags" : "· prose"}
          </option>
        ))}
      </select>
    </label>
  )}
  {captionOn && chosen?.supports_escalation && (
    /* the existing escalate checkbox + every-Nth field, unchanged */
  )}
</fieldset>
```

  - Add a **mode** select (`Frames (stills from video + images)` / `Clips (whole videos, for video models)`) and, in the grid of numbers, `Max frames per clip` (0 = unlimited, default 40) and `Min clip length (s)` (default 2, shown only in clips mode). `start()` sends `mode`, `captioner: captionerId`, `max_frames_per_clip`, `min_clip_secs`; drop `escalate` to `false` when the chosen captioner cannot escalate.

- [ ] **Step 2: Dataset picker + trigger word.** Replace "Recent runs" (job list) with `useDatasets()` — chips labelled `name · mode · n frames`; selecting one sets `activeDatasetId`. Frames come from `useDatasetFramesForDataset(activeDatasetId)`. The running job's live status still comes from the job (`dataset.prep_job_id`). Above the grid, an inline field **Trigger word** bound to `dataset.trigger_word` → `updateDataset(id, { trigger_word })` on blur, with `tokenWarning`-style hint text reused from the concept panel (call the same pure check client-side: a small `tokenWarning(token)` in `ui/src/features/dataset/tokens.ts` with the same word list as `compose.rs`, so the UI warns instantly and the server is the source of truth on the concept list).

- [ ] **Step 3: Rejection chips.** Compute counts per `rejection_reason` over the frame list; render a chip row: `Kept (n)` `Black (n)` `Transition (n)` `Blur (n)` `Duplicate (n)` `Cap (n)` `Unusable (n)`; one active filter at a time (default Kept). Rejected `FrameCard`s show the reason as a badge and a **Restore** button (`updateDatasetFrame(id, { restore: true })`); the Exclude checkbox is hidden for rejected frames. Kept count for export = `rejection_reason === "" && !excluded`.

- [ ] **Step 4: `ConceptsPanel.tsx`** — props `{ datasetId, concepts, onChanged }`: list rows (`name`, `token` in mono, `frame_count`, warning text in the risk colour when `token_warning`, a `< 20 examples` warning when `frame_count < 20`, Delete), and a create form (name, token, description). Under the list, the fixed reminder: *"Wide shots teach position, close-ups teach form — mix both, and vary the backgrounds."* Mounted in the result column above the grid once a dataset is selected.

- [ ] **Step 5: Multi-select in the grid.** `FrameCard` gains a `selected` checkbox (top-left) and the grid a toolbar when ≥1 selected: `Assign to <concept select> ` / `Remove from <concept select>` / `Clear selection` → `assignConcept`/`unassignConcept` with the selected ids, then refetch concepts + map. Each card shows its concept tokens as small chips (from `useFrameConceptMap`).

- [ ] **Step 6: Export** — add a `Caption order` select (`Prose first (FLUX.2)` / `Tags first (Anime/SDXL)`) and call `exportDatasetById(datasetId, destDir, order)`.

- [ ] **Step 7: Live verify** against dev-mock (`pnpm dev` from *this worktree's* `ui/`, on a port nobody else uses — the browser tool's `preview_start(name)` resolves `.claude/launch.json` against the main checkout, so start Vite directly and navigate to its port): screenshot (a) the captioning section with the recommendation text and the tagger selected, (b) the chip row with a rejected frame restored, (c) two frames selected and assigned to a concept, the concept panel showing count 2 and a warning for token `anime`, (d) export with tags-first. `pnpm typecheck && pnpm lint && pnpm build` clean.

- [ ] **Step 8: Commit**

```bash
git add ui/src/features/dataset
git commit -m "feat(ui): dataset tab — captioner choice, modes, rejection chips, datasets, concepts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Learn mode (`LearnSets.tsx`)

**Files:**
- Create: `ui/src/features/dataset/LearnSets.tsx`
- Modify: `ui/src/features/dataset/Dataset.tsx` (a `Learn` / `Grid` toggle in the result column), `dataset.css`

- [ ] **Step 1: Set builder (pure, unit-testable in TS)** — in `LearnSets.tsx` export:

```ts
export type SetGrouping = "clip" | "similarity";
export const SET_SIZE = 30;

/** Groups kept frames into sets of at most SET_SIZE. "clip": consecutive
 *  frames of the same source_path. "similarity": greedy buckets over the
 *  provided hash distance function (a frame joins the first bucket whose
 *  seed is within `maxDistance`). Hashes are not sent to the UI, so
 *  similarity grouping uses the tag + source as a coarse proxy on the
 *  client; server-side hashing is a follow-up if this proves too coarse. */
export function buildSets(frames: DatasetFrame[], grouping: SetGrouping): DatasetFrame[][] {
  const kept = frames.filter((f) => f.rejection_reason === "" && !f.excluded);
  const key = (f: DatasetFrame) => (grouping === "clip" ? f.source_path : `${f.tag}::${f.source_path}`);
  const buckets = new Map<string, DatasetFrame[]>();
  for (const f of kept) {
    const k = key(f);
    if (!buckets.has(k)) buckets.set(k, []);
    buckets.get(k)!.push(f);
  }
  const sets: DatasetFrame[][] = [];
  for (const bucket of buckets.values()) {
    for (let i = 0; i < bucket.length; i += SET_SIZE) sets.push(bucket.slice(i, i + SET_SIZE));
  }
  return sets;
}
```

(The spec's similarity grouping via perceptual hash needs the hash on the frame row; that column is deliberately left for Plan 2 — record this in `docs/TODO.md` in Task 14. The toggle still exists so the UI shape is final.)

- [ ] **Step 2: Component** — props `{ datasetId, frames, concepts, conceptMap, imageUrlFor, onChanged }`. State: `grouping`, `setIndex`, `selected: Set<string>`, `conceptId`, `description`. Renders: header `Set {i+1} / {n} · {frames.length} frames · {source file name}`, a grid of thumbnails with a selected outline (click toggles; `Shift`+click selects the range from the last clicked), buttons **All in set** / **Invert selection** / **Clear**, a concept select with **+ New concept** (inline name/token/description mini-form that calls `createConcept`), **Assign** (`assignConcept(conceptId, [...selected])`) and **Remove** (`unassignConcept`), then **← Previous** / **Next →**. Keyboard: `←`/`→` change set, `a` selects all, `i` inverts, `Escape` clears, `Enter` assigns to the current concept. Each thumbnail shows its current concept tokens as chips. A right-hand summary lists every concept with `frame_count`, the `< 20` warning and `token_warning`, and the fixed reminder sentence.

- [ ] **Step 3: Live verify** — screenshots: navigate sets with the keyboard, select 3 frames, create concept `kenji` / `kenji_xy`, assign, see count 3 and chips; remove one, see count 2; a concept with token `foot` shows its warning. `pnpm typecheck && pnpm lint && pnpm build` clean.

- [ ] **Step 4: Commit**

```bash
git add ui/src/features/dataset
git commit -m "feat(ui): guided Learn mode — frame sets, multi-select concept assignment, per-concept summary

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: Clip-mode UI, docs, final gates

**Files:**
- Modify: `ui/src/features/dataset/Dataset.tsx`, `dataset.css`, `docs/TODO.md`

- [ ] **Step 1: Clip mode in the grid** — when `dataset.mode === "clips"`, `FrameCard` renders the preview still, `duration_secs` formatted `m:ss`, the source file name, and two number inputs **Start (s)** / **End (s)** committed via `updateDatasetFrame(id, { clip_start_secs, clip_end_secs })` (blank = whole clip); `Unusable` rejected clips show their badge with no thumbnail. Export button text becomes `Export n clip(s)`.

- [ ] **Step 2: `docs/TODO.md`** — in the "Lokale KI-Trainings-Engine" section, add a "Teilsystem 1 — Erweiterungen (Plan 1) ✅" block in the existing German/✅ style covering: datasets as objects (migration 0015), rejected frames with reasons + restore, filter level C, captioning optional with the recommendation text, captioner registry (Florence-2 + WD tagger, JoyCaption deferred with the llama.cpp `--mmproj` check still open), concepts + learn mode, clip mode, composed export with order; and the explicit leftovers: similarity grouping needs a stored hash column (Plan 2), JoyCaption, Wan-clip training itself. Reference the spec and this plan by path.

- [ ] **Step 3: Full gates on the worktree**

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd sidecar && uv run ruff check . && uv run pytest -q && cd ..
cd ui && pnpm typecheck && pnpm lint && pnpm build && cd ..
```

Expected: all clean. Report the exact Rust test count.

- [ ] **Step 4: Commit**

```bash
git add ui/src/features/dataset docs/TODO.md
git commit -m "feat(ui): clip-mode curation; docs: record dataset extensions (Plan 1) as shipped

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 5: Hand back** — use `superpowers:finishing-a-development-branch`: the orchestrator re-verifies gates on the branch, re-reads `filter.rs`/`compose.rs`/`export_dataset`, re-downloads and re-hashes the two tagger files against the catalog pin, then merges `--no-ff` to `master` and pushes (the pre-push hook re-runs every gate).

---

## Self-review (done while writing)

- **Spec coverage:** 3 (datasets table, rejection reasons, optional captioning, filter C, clip mode) → Tasks 1, 2, 4, 10, 14; 3A concepts → Tasks 3, 11, 12; 3C recommendation text → Task 12; 3D registry + WD tagger + order-by-profile → Tasks 6, 7, 8, 5 (order is a request field now; Plan 2 wires the profile default); 3E not-included → recorded in Task 14 docs; 4B learn sets → Task 13 (similarity grouping by real hash deferred and recorded); 5 tests → every task has failing-first tests, Task 11 has the HTTP integration test, Task 7 Step 5 is the one real tagger run.
- **Placeholders:** the only `<paste …>` markers are in Task 6 Step 3 and are the deliberate "compute the hash, never guess it" instruction; every other code block is complete.
- **Type consistency:** `DatasetMode` (db) is reused by the request; `RejectionReason::as_str` values match the migration comment and the UI chip labels; `CaptionOrder` serialises as `tags_first`/`prose_first` on both sides; `Captioner.style: CaptionStyle` is the same enum `compose_caption` takes; `caption_with` returns the engine label that `captioner::find(&frame.caption_engine)` looks up at export (`"florence2"` and `"wd-eva02-tagger-v3"` are the registry ids, and the sidecar returns exactly those).
