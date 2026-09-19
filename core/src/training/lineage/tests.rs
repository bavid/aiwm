#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use super::{is_library_lora, lineage, list_loras, LoraLineage, LoraSummary};
use crate::db::{
    Database, DatasetMode, NewDataset, NewJob, NewModel, NewTrainingRun, Preset, RunState,
};

/// A fresh in-memory store plus the two roots the overview reads files
/// from: the model store (LoRA-ness) and the training root (samples).
struct Fx {
    _tmp: tempfile::TempDir,
    db: Database,
    store: PathBuf,
    training: PathBuf,
}

async fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    let training = tmp.path().join("training");
    std::fs::create_dir_all(store.join("image").join("loras")).unwrap();
    std::fs::create_dir_all(&training).unwrap();
    Fx {
        db: Database::connect_in_memory().await.unwrap(),
        store,
        training,
        _tmp: tmp,
    }
}

/// A minimal `.safetensors` whose header carries one `lora_A` down
/// projection of `rank`.
fn write_lora_weights(path: &Path, rank: u32) {
    let header = serde_json::json!({
        "transformer.single_transformer_blocks.0.attn.to_q.lora_A.weight": {
            "dtype": "BF16", "shape": [rank, 3072], "data_offsets": [0, 8]
        },
        "transformer.single_transformer_blocks.0.attn.to_q.lora_B.weight": {
            "dtype": "BF16", "shape": [3072, rank], "data_offsets": [8, 16]
        }
    });
    let json = serde_json::to_vec(&header).unwrap();
    let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(&json);
    bytes.extend(std::iter::repeat_n(0u8, 16));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A model row filed at `path` with the given provenance; no file is written.
async fn model_at(fx: &Fx, name: &str, path: &Path, source: &str) -> String {
    fx.db
        .models()
        .insert(NewModel {
            name: name.into(),
            family: Some("flux2".into()),
            format: "safetensors".into(),
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: 4096,
            source: source.into(),
            roles: vec!["lora".into()],
            ..NewModel::default()
        })
        .await
        .unwrap()
        .id
}

/// A library LoRA row (under `<store>/image/loras`) with no file on disk.
async fn lora(fx: &Fx, name: &str, source: &str) -> String {
    let path = fx
        .store
        .join("image")
        .join("loras")
        .join(format!("{name}.safetensors"));
    model_at(fx, name, &path, source).await
}

/// A dataset whose prep job used `captioner`.
async fn dataset(fx: &Fx, name: &str, captioner: Option<&str>) -> String {
    let job = fx
        .db
        .jobs()
        .insert(NewJob {
            params: serde_json::json!({ "root": "E:\\raw\\clips", "captioner": captioner }),
            ..NewJob::new("dataset_prep")
        })
        .await
        .unwrap();
    fx.db
        .datasets()
        .create(NewDataset {
            name: name.into(),
            mode: DatasetMode::Frames,
            source_root: "E:\\raw\\clips".into(),
            prep_job_id: Some(job.id),
            work_dir: None,
        })
        .await
        .unwrap()
        .id
}

/// A run in `state` with `step` steps done, started from `init` (a LoRA
/// id) or from scratch, on `dataset`.
async fn run(
    fx: &Fx,
    name: &str,
    init: Option<&str>,
    dataset: Option<&str>,
    state: RunState,
    step: i64,
    image_count: Option<i64>,
) -> String {
    let created = fx
        .db
        .training_runs()
        .create(NewTrainingRun {
            name: name.into(),
            profile_family: "flux2-klein-4b".into(),
            target_model_id: None,
            dataset_id: dataset.map(str::to_string),
            data_kind: DatasetMode::Frames,
            trigger_word: "ghibli_xy".into(),
            preset: Preset::Balanced,
            hyperparams_json: "{\"rank\":16,\"steps\":1500}".into(),
            sample_prompts_json: "[]".into(),
            work_dir: String::new(),
            init_lora_model_id: init.map(str::to_string),
            image_count,
        })
        .await
        .unwrap();
    let runs = fx.db.training_runs();
    runs.set_progress(&created.id, step, 1500, None)
        .await
        .unwrap();
    let path: &[RunState] = match state {
        RunState::Preparing => &[],
        RunState::Running => &[RunState::Running],
        RunState::Completed => &[RunState::Running, RunState::Finishing, RunState::Completed],
        RunState::Cancelled => &[RunState::Running, RunState::Cancelled],
        RunState::Failed => &[RunState::Running, RunState::Failed],
        other => panic!("the fixture does not build a {other:?} run"),
    };
    for next in path {
        runs.set_state(&created.id, *next).await.unwrap();
    }
    created.id
}

/// The LoRA `run` produced: a library row with `source = training:<run>`
/// and the run's `result_model_id` pointing back.
async fn result_of(fx: &Fx, run_id: &str, name: &str) -> String {
    let id = lora(fx, name, &format!("training:{run_id}")).await;
    fx.db.training_runs().set_result(run_id, &id).await.unwrap();
    id
}

async fn lineage_of(fx: &Fx, model_id: &str) -> Option<LoraLineage> {
    lineage(&fx.db, &fx.store, &fx.training, model_id)
        .await
        .unwrap()
}

async fn summary_of(fx: &Fx, model_id: &str) -> LoraSummary {
    list_loras(&fx.db, &fx.store)
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.model_id == model_id)
        .expect("the LoRA is listed")
}

#[test]
fn a_lora_is_a_file_under_the_store_loras_folder() {
    let store = Path::new("E:\\AI\\models");
    assert!(is_library_lora(
        store,
        "E:\\AI\\models\\image\\loras\\style.safetensors"
    ));
    assert!(!is_library_lora(
        store,
        "E:\\AI\\models\\image\\checkpoints\\base.safetensors"
    ));
    assert!(!is_library_lora(
        store,
        "D:\\elsewhere\\image\\loras\\style.safetensors"
    ));
}

#[tokio::test]
async fn a_three_run_chain_is_walked_oldest_first() {
    let fx = fixture().await;
    let ds1 = dataset(&fx, "Ghibli stills", Some("florence2")).await;
    let ds2 = dataset(&fx, "Ghibli clips", None).await;
    let r1 = run(
        &fx,
        "v1",
        None,
        Some(&ds1),
        RunState::Completed,
        1500,
        Some(400),
    )
    .await;
    let a = result_of(&fx, &r1, "Ghibli v1").await;
    let r2 = run(
        &fx,
        "v2",
        Some(&a),
        Some(&ds2),
        RunState::Completed,
        1000,
        Some(250),
    )
    .await;
    let b = result_of(&fx, &r2, "Ghibli v2").await;
    let r3 = run(&fx, "v3", Some(&b), None, RunState::Running, 300, None).await;
    let c = result_of(&fx, &r3, "Ghibli v3").await;

    let got = lineage_of(&fx, &c).await.expect("a library LoRA");

    let ids: Vec<&str> = got.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec![r1.as_str(), r2.as_str(), r3.as_str()]);
    assert_eq!(got.lora.model_id, c);

    let first = &got.runs[0];
    assert_eq!(first.init_lora_model_id, None);
    assert_eq!(first.result_model_name.as_deref(), Some("Ghibli v1"));
    let ds = first.dataset.as_ref().expect("dataset");
    assert_eq!(ds.name, "Ghibli stills");
    assert_eq!(ds.source_root, "E:\\raw\\clips");
    assert_eq!(ds.captioner.as_deref(), Some("florence2"));
    assert_eq!(first.hyperparams.and_then(|h| h.rank), Some(16));

    let second = &got.runs[1];
    assert_eq!(second.init_lora_model_id.as_deref(), Some(a.as_str()));
    assert_eq!(second.init_lora_name.as_deref(), Some("Ghibli v1"));
    assert_eq!(second.dataset.as_ref().unwrap().captioner, None);

    let third = &got.runs[2];
    assert_eq!(third.state, RunState::Running);
    assert_eq!(third.dataset, None, "a run without a dataset row");
    assert_eq!(third.image_count, None);
}

#[tokio::test]
async fn a_cycle_between_two_loras_terminates() {
    // Corrupted rows: A came out of r1 which started from B, and B came out
    // of r2 which started from A. Impossible in practice, but the walk must
    // not spin on it.
    let fx = fixture().await;
    let a = lora(&fx, "A", "manual").await;
    let b = lora(&fx, "B", "manual").await;
    let r1 = run(&fx, "r1", Some(&b), None, RunState::Completed, 100, Some(1)).await;
    let r2 = run(&fx, "r2", Some(&a), None, RunState::Completed, 100, Some(1)).await;
    fx.db
        .models()
        .set_family_and_source(&a, Some("flux2"), &format!("training:{r1}"))
        .await
        .unwrap();
    fx.db
        .models()
        .set_family_and_source(&b, Some("flux2"), &format!("training:{r2}"))
        .await
        .unwrap();

    let got = lineage_of(&fx, &a).await.expect("a library LoRA");

    let ids: Vec<&str> = got.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec![r2.as_str(), r1.as_str()]);
    assert_eq!(summary_of(&fx, &a).await.runs, 2);
}

#[tokio::test]
async fn a_missing_link_ends_the_chain() {
    let fx = fixture().await;
    // B claims to come from a run that was deleted from the history.
    let b = lora(&fx, "B", "training:gone").await;
    let r3 = run(
        &fx,
        "r3",
        Some(&b),
        None,
        RunState::Completed,
        500,
        Some(10),
    )
    .await;
    let c = result_of(&fx, &r3, "C").await;

    let got = lineage_of(&fx, &c).await.expect("a library LoRA");

    let ids: Vec<&str> = got.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec![r3.as_str()]);
    assert_eq!(got.runs[0].init_lora_name.as_deref(), Some("B"));
    assert_eq!(summary_of(&fx, &b).await.runs, 0);
}

#[tokio::test]
async fn an_imported_lora_is_listed_as_not_trained_with_no_lineage() {
    let fx = fixture().await;
    let id = lora(&fx, "Downloaded style", "civitai:12345").await;

    let summary = summary_of(&fx, &id).await;
    assert!(!summary.trained);
    assert_eq!(summary.runs, 0);
    assert_eq!(summary.total_steps, 0);
    assert_eq!(summary.total_images, None);
    assert_eq!(summary.size_bytes, 4096);
    assert_eq!(summary.family.as_deref(), Some("flux2"));

    let got = lineage_of(&fx, &id).await.expect("a library LoRA");
    assert!(got.runs.is_empty());
    assert_eq!(got.lora, summary);
}

#[tokio::test]
async fn a_model_outside_the_loras_folder_is_neither_listed_nor_a_lineage() {
    let fx = fixture().await;
    let checkpoint = fx
        .store
        .join("image")
        .join("checkpoints")
        .join("base.safetensors");
    let id = model_at(&fx, "Base", &checkpoint, "manual").await;
    lora(&fx, "Real LoRA", "manual").await;

    let listed = list_loras(&fx.db, &fx.store).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Real LoRA");
    assert!(lineage_of(&fx, &id).await.is_none());
    assert!(lineage_of(&fx, "no-such-model").await.is_none());
}

#[tokio::test]
async fn aggregates_count_only_completed_steps_and_need_every_image_count() {
    let fx = fixture().await;
    let r1 = run(&fx, "r1", None, None, RunState::Completed, 1500, Some(400)).await;
    let a = result_of(&fx, &r1, "A").await;
    let r2 = run(
        &fx,
        "r2",
        Some(&a),
        None,
        RunState::Cancelled,
        700,
        Some(250),
    )
    .await;
    let b = result_of(&fx, &r2, "B").await;
    let r3 = run(&fx, "r3", Some(&b), None, RunState::Completed, 1000, None).await;
    let c = result_of(&fx, &r3, "C").await;

    let of_b = summary_of(&fx, &b).await;
    assert!(of_b.trained);
    assert_eq!(of_b.runs, 2);
    assert_eq!(
        of_b.total_steps, 1500,
        "the cancelled run's steps do not count"
    );
    assert_eq!(of_b.total_images, Some(650));

    let of_c = summary_of(&fx, &c).await;
    assert_eq!(of_c.runs, 3);
    assert_eq!(of_c.total_steps, 2500);
    assert_eq!(of_c.total_images, None, "one run has no count");
}

#[tokio::test]
async fn the_list_is_newest_first() {
    let fx = fixture().await;
    let older = lora(&fx, "older", "manual").await;
    let newer = lora(&fx, "newer", "manual").await;
    // `imported_at` has second resolution; make the order unambiguous.
    sqlx_set_imported_at(&fx, &older, "2026-01-01T00:00:00Z").await;
    sqlx_set_imported_at(&fx, &newer, "2026-02-01T00:00:00Z").await;

    let ids: Vec<String> = list_loras(&fx.db, &fx.store)
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.model_id)
        .collect();

    assert_eq!(ids, vec![newer, older]);
}

async fn sqlx_set_imported_at(fx: &Fx, id: &str, when: &str) {
    sqlx::query("UPDATE models SET imported_at = $1 WHERE id = $2")
        .bind(when)
        .bind(id)
        .execute(fx.db.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn rank_comes_from_the_header_and_is_none_when_the_file_cannot_be_read() {
    let fx = fixture().await;
    let readable = lora(&fx, "readable", "manual").await;
    let path = fx
        .db
        .models()
        .get(&readable)
        .await
        .unwrap()
        .unwrap()
        .file_path;
    write_lora_weights(Path::new(&path), 32);
    let missing = lora(&fx, "missing", "manual").await;
    let garbage = lora(&fx, "garbage", "manual").await;
    let garbage_path = fx
        .db
        .models()
        .get(&garbage)
        .await
        .unwrap()
        .unwrap()
        .file_path;
    std::fs::write(&garbage_path, b"not a safetensors file").unwrap();

    assert_eq!(summary_of(&fx, &readable).await.rank, Some(32));
    assert_eq!(summary_of(&fx, &missing).await.rank, None);
    assert_eq!(summary_of(&fx, &garbage).await.rank, None);
}

#[tokio::test]
async fn a_run_reports_its_duration_and_latest_samples() {
    let fx = fixture().await;
    let r1 = run(&fx, "v1", None, None, RunState::Completed, 1500, Some(400)).await;
    sqlx::query("UPDATE training_runs SET started_at = $1, finished_at = $2 WHERE id = $3")
        .bind("2026-09-19T10:00:00Z")
        .bind("2026-09-19T11:30:00Z")
        .bind(&r1)
        .execute(fx.db.pool())
        .await
        .unwrap();
    let a = result_of(&fx, &r1, "A").await;
    // The samples live where the runner puts them: `<training>/<run
    // id>/output/<run name>/samples`, next to the checkpoint they belong to.
    let out = fx.training.join(&r1).join("output").join("v1");
    std::fs::create_dir_all(out.join("samples")).unwrap();
    std::fs::write(out.join("v1_000001500.safetensors"), b"").unwrap();
    std::fs::write(
        out.join("samples").join("1789641631350__000001500_0.jpg"),
        b"",
    )
    .unwrap();
    std::fs::write(
        out.join("samples").join("1789641631350__000001500_1.jpg"),
        b"",
    )
    .unwrap();
    std::fs::write(
        out.join("samples").join("1789641631000__000001000_0.jpg"),
        b"",
    )
    .unwrap();

    let got = lineage_of(&fx, &a).await.expect("a library LoRA");

    assert_eq!(got.runs[0].duration_secs, Some(90 * 60));
    assert_eq!(got.runs[0].samples, vec!["0".to_string(), "1".to_string()]);
}
