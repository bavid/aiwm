//! The apply over a fixture with every group populated: a dry run deletes
//! nothing and lists exactly what would go; a real run deletes exactly that
//! list, keeps everything protected, removes the rows and writes the log;
//! unknown ids do nothing.

use std::path::PathBuf;

use super::super::scan::tests::{fixture, touch, touch_secs, Fx};
use super::tests::{everything, reasons, run, sel, tree};
use super::*;
use crate::cleanup::RetentionPolicy;
use crate::db::{Dataset, RunState, TrainingRun};
use crate::orchestrator::JobState;

/// The scan fixture with every group populated, as the scan tests lay it
/// out (one dataset with kept, discarded and missing frames; a completed
/// run whose LoRA is in the library and a failed one without; caches,
/// staging, ComfyUI leftovers, pending import, old logs, backups, media
/// beyond a 30-day rule and an orphan).
struct Full {
    fx: Fx,
    dataset: Dataset,
    whole_run: TrainingRun,
    whole_dir: PathBuf,
    partial_run: TrainingRun,
    partial_dir: PathBuf,
}

async fn full() -> Full {
    let mut fx = fixture().await;
    fx.ctx.policy = RetentionPolicy {
        max_age_days: 30,
        max_total_mb: 0,
    };
    let p = fx.paths().clone();
    let outputs = p.outputs_dir();
    let old = outputs.join("old.png");
    let fresh = outputs.join("fresh.png");
    touch(&old, 1_000, 40);
    touch(&fresh, 1_000, 2);
    fx.job(JobState::Completed, Some(&old)).await;
    fx.job(JobState::Completed, Some(&fresh)).await;
    touch(&outputs.join("stray.mp4"), 500, 1);

    let job = fx.job(JobState::Cancelled, None).await;
    let dataset = fx.dataset("Demo", &job).await;
    let work = p.datasets_dir().join(&job).join("raw");
    fx.frame(&dataset, &work.join("kept.png"), Some(100), "", false)
        .await;
    fx.frame(
        &dataset,
        &work.join("blurry.png"),
        Some(200),
        "blurry",
        false,
    )
    .await;
    fx.frame(&dataset, &work.join("excluded.png"), Some(300), "", true)
        .await;
    fx.frame(&dataset, &work.join("never-written.png"), None, "", false)
        .await;
    let root = p.datasets_dir();
    touch(&root.join("gone-dataset").join("raw").join("b.png"), 400, 1);
    touch(
        &root.join("gone-dataset").join("previews").join("b.jpg"),
        40,
        1,
    );

    let placeholder = p.training_dir().join("placeholder");
    let whole_run = fx.run("lora-v1", RunState::Completed, &placeholder).await;
    let whole_dir = fx.run_folder(&whole_run.id, "lora-v1");
    fx.db
        .training_runs()
        .set_work_dir(&whole_run.id, &whole_dir.to_string_lossy())
        .await
        .unwrap();
    let lora = fx
        .ctx
        .store
        .join("image")
        .join("loras")
        .join("lora-v1.safetensors");
    let model = fx.model("lora-v1", &lora, Some(800)).await;
    fx.db
        .training_runs()
        .set_result(&whole_run.id, &model.id)
        .await
        .unwrap();
    let partial_run = fx.run("lora-v2", RunState::Failed, &placeholder).await;
    let partial_dir = fx.run_folder(&partial_run.id, "lora-v2");
    fx.db
        .training_runs()
        .set_work_dir(&partial_run.id, &partial_dir.to_string_lossy())
        .await
        .unwrap();

    touch(&p.cache_dir().join("registry").join("index.json"), 1_000, 1);
    touch(&p.downloads_dir().join("stale-id").join("w.part"), 2_000, 9);
    let comfy = p.comfyui_data_dir();
    touch_secs(&comfy.join("input").join("old.png"), 300, 2 * 3_600);
    touch_secs(&comfy.join("temp").join("preview.png"), 400, 2 * 3_600);
    touch(&p.pending_import_dir().join("aiwm.db"), 5_000, 1);
    let logs = p.logs_dir();
    touch(&logs.join("aiwm.log.2026-01-01"), 100, 60);
    touch(&logs.join("aiwm.log.2026-02-01"), 100, 45);
    touch(&logs.join("aiwm.log"), 100, 0);
    let exports = p.exports_dir();
    touch(&exports.join("aiwm-backup-1.zip"), 1_000, 20);
    touch(&exports.join("aiwm-backup-2.zip"), 2_000, 1);
    let whole_run = fx
        .db
        .training_runs()
        .get(&whole_run.id)
        .await
        .unwrap()
        .unwrap();
    let partial_run = fx
        .db
        .training_runs()
        .get(&partial_run.id)
        .await
        .unwrap()
        .unwrap();
    Full {
        fx,
        dataset,
        whole_run,
        whole_dir,
        partial_run,
        partial_dir,
    }
}

impl Full {
    async fn frames_left(&self) -> Vec<String> {
        self.fx
            .db
            .dataset_frames()
            .list_for_dataset(&self.dataset.id)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.frame_path)
            .collect()
    }
}

#[tokio::test]
async fn dry_run_deletes_nothing_for_every_group_and_lists_exactly_what_would_go() {
    let f = full().await;
    let report = f.fx.scan().await;
    assert!(
        report.groups.iter().all(|g| !g.entries.is_empty()),
        "the fixture must populate every group: {:?}",
        report
            .groups
            .iter()
            .map(|g| (&g.key, g.entries.len()))
            .collect::<Vec<_>>()
    );
    let before = tree(f.fx.tmp.path());
    let frames_before = f.frames_left().await;

    let r = run(&f.fx, true, everything(&report)).await.unwrap();

    assert!(r.dry_run);
    assert_eq!(tree(f.fx.tmp.path()), before, "a dry run deletes nothing");
    assert_eq!(f.frames_left().await, frames_before, "and removes no rows");
    assert!(f.fx.db.cleanup_log().list(20).await.unwrap().is_empty());
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    // What would go is what the scan offered, group by group.
    for g in &report.groups {
        let entries: Vec<&EntryResult> = r.entries.iter().filter(|e| e.group == g.key).collect();
        assert_eq!(entries.len(), g.entries.len(), "{}", g.key);
        let files: u64 = entries.iter().map(|e| e.files).sum();
        let bytes: u64 = entries.iter().map(|e| e.bytes).sum();
        let rows: u64 = entries.iter().map(|e| e.rows).sum();
        assert_eq!((files, bytes), (g.total_files, g.total_bytes), "{}", g.key);
        assert_eq!(
            rows,
            g.entries.iter().map(|e| e.rows).sum::<u64>(),
            "{}",
            g.key
        );
        for e in entries {
            assert_eq!(
                e.paths.len() as u64,
                e.files,
                "{}/{}: {:?}",
                g.key,
                e.id,
                e.paths
            );
            for p in &e.paths {
                assert!(before.contains_key(p), "{p} must be a real file");
            }
        }
    }
    assert_eq!(
        r.removed_rows, 3,
        "two discarded frames and one missing row"
    );
}

#[tokio::test]
async fn a_real_apply_deletes_exactly_the_dry_run_list_for_every_group() {
    let f = full().await;
    let report = f.fx.scan().await;
    let before = tree(f.fx.tmp.path());
    let dry = run(&f.fx, true, everything(&report)).await.unwrap();

    let real = run(&f.fx, false, everything(&report)).await.unwrap();

    assert!(!real.dry_run);
    assert!(real.skipped.is_empty(), "{:?}", real.skipped);
    let doomed: Vec<&String> = dry.entries.iter().flat_map(|e| &e.paths).collect();
    let mut expected = before.clone();
    for p in &doomed {
        assert!(expected.remove(*p).is_some(), "{p} listed twice or unknown");
    }
    assert_eq!(tree(f.fx.tmp.path()), expected, "exactly the preview went");
    assert_eq!(
        (real.deleted_files, real.freed_bytes, real.removed_rows),
        (dry.deleted_files, dry.freed_bytes, dry.removed_rows)
    );
    for (d, r) in dry.entries.iter().zip(&real.entries) {
        assert_eq!((&d.group, &d.id), (&r.group, &r.id));
        assert_eq!(
            (d.files, d.bytes, d.rows),
            (r.files, r.bytes, r.rows),
            "{}",
            d.id
        );
    }
    // What must still be there.
    let p = f.fx.paths();
    for kept in [
        p.outputs_dir().join("fresh.png"),
        p.datasets_dir()
            .join(f.dataset.prep_job_id.as_deref().unwrap())
            .join("raw")
            .join("kept.png"),
        f.partial_dir
            .join("output")
            .join("lora-v2")
            .join("lora-v2.safetensors"),
        f.partial_dir.join("config.yaml"),
        p.logs_dir().join("aiwm.log"),
        f.fx.ctx
            .store
            .join("image")
            .join("loras")
            .join("lora-v1.safetensors"),
        f.fx.source_dir(),
    ] {
        assert!(kept.exists(), "{} must survive", kept.display());
    }
    assert!(!f.whole_dir.exists(), "the whole run folder went");
    assert!(!p.datasets_dir().join("gone-dataset").exists());
    assert!(!p.downloads_dir().join("stale-id").exists());
    assert!(
        p.cache_dir().is_dir(),
        "the cache root is emptied, not removed"
    );
    let left = f.frames_left().await;
    assert_eq!(left.len(), 1);
    assert!(left[0].ends_with("kept.png"), "{left:?}");
    // The whole folder's row is the one whose folder is gone.
    assert!(f.whole_run.result_model_id.is_some());
    assert!(!f.partial_run.state.is_terminal() || f.partial_dir.is_dir());
    // One log row per entry, newest first, with the entry's numbers.
    let log = f.fx.db.cleanup_log().list(50).await.unwrap();
    assert_eq!(log.len(), real.entries.len());
    for e in &real.entries {
        let row = log
            .iter()
            .find(|l| l.group_key == e.group && l.entry_id == e.id)
            .unwrap_or_else(|| panic!("no log row for {}/{}", e.group, e.id));
        assert_eq!(
            (
                row.deleted_files,
                row.freed_bytes,
                row.removed_rows,
                row.skipped_count
            ),
            (e.files, e.bytes, e.rows, 0),
            "{}",
            e.id
        );
        assert_eq!(row.entry_label, e.label);
        assert!(row.detail["paths"].is_array());
    }
}

#[tokio::test]
async fn unknown_ids_are_skipped_with_a_reason_and_an_unknown_group_is_an_error() {
    let f = full().await;
    let before = tree(f.fx.tmp.path());
    let selections: Vec<Selection> = crate::cleanup::scan::GROUP_KEYS
        .iter()
        .map(|g| sel(g, &["nope", "..\\nope"]))
        .collect();

    let r = run(&f.fx, false, selections).await.unwrap();

    assert_eq!(tree(f.fx.tmp.path()), before);
    assert_eq!((r.deleted_files, r.freed_bytes, r.removed_rows), (0, 0, 0));
    assert_eq!(r.entries.len(), 18);
    for e in &r.entries {
        assert_eq!(reasons(e), [SKIP_NOT_OFFERED], "{}/{}", e.group, e.id);
    }
    assert_eq!(r.skipped.len(), 18);

    let err = run(&f.fx, true, vec![sel("models", &["x"])])
        .await
        .expect_err("unknown group");
    assert!(err.to_string().contains("unknown cleanup group"), "{err}");
}
