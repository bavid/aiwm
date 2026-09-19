//! Dataset housekeeping over a real loopback server: usage, bulk keep/
//! discard, frame deletion, cleanup, global dedup and dataset deletion —
//! status codes (incl. the 400 refusal while a training run is active and 404
//! for an unknown dataset) and the JSON shapes the UI reads.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::db::{
    DatasetMode, JobPatch, NewDataset, NewDatasetFrame, NewJob, NewTrainingRun, Preset,
};
use aiwm_core::orchestrator::JobState;
use aiwm_core::{ApiServer, App, AppPaths};
use serde_json::{json, Value};

struct Fx {
    server: ApiServer,
    _tmp: tempfile::TempDir,
    app: Arc<App>,
    dataset_id: String,
    /// kept (100 B), excluded (200 B), rejected as blur (300 B)
    frames: Vec<String>,
    frame_files: Vec<PathBuf>,
    work_root: PathBuf,
}

impl Fx {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.server.addr)
    }
}

async fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let app = Arc::new(App::load(AppPaths::rooted(tmp.path())).await.unwrap());
    let job = app
        .db
        .jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap();
    // A finished prep job: while it runs, deleting is refused (tested below).
    app.db
        .jobs()
        .set_state(&job.id, JobState::Cancelled, JobPatch::default())
        .await
        .unwrap();
    let dataset = app
        .db
        .datasets()
        .create(NewDataset {
            name: "Demo".into(),
            mode: DatasetMode::Frames,
            source_root: tmp.path().join("src").to_string_lossy().into_owned(),
            prep_job_id: Some(job.id.clone()),
            work_dir: None,
        })
        .await
        .unwrap();
    let work_root = app.paths.outputs_dir().join("datasets").join(&job.id);
    let clip_dir = work_root.join("raw").join("Tag").join("clip");
    std::fs::create_dir_all(&clip_dir).unwrap();

    let mut frames = Vec::new();
    let mut frame_files = Vec::new();
    for (i, (bytes, reason, excluded)) in [(100, "", false), (200, "", true), (300, "blur", false)]
        .into_iter()
        .enumerate()
    {
        let path = clip_dir.join(format!("frame_{i:06}.png"));
        std::fs::write(&path, vec![1u8; bytes]).unwrap();
        let f = app
            .db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job.id.clone(),
                dataset_id: Some(dataset.id.clone()),
                tag: "Tag".into(),
                source_path: tmp
                    .path()
                    .join("src")
                    .join("clip.mp4")
                    .to_string_lossy()
                    .into(),
                frame_path: path.to_string_lossy().into_owned(),
                timestamp_secs: Some(i as f64),
                rejection_reason: reason.into(),
                duration_secs: None,
            })
            .await
            .unwrap();
        if excluded {
            app.db
                .dataset_frames()
                .set_excluded(&f.id, true)
                .await
                .unwrap();
        }
        frames.push(f.id);
        frame_files.push(path);
    }

    let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    Fx {
        server,
        _tmp: tmp,
        app,
        dataset_id: dataset.id,
        frames,
        frame_files,
        work_root,
    }
}

async fn post(fx: &Fx, path: &str, body: Value) -> (u16, Value) {
    let r = reqwest::Client::new()
        .post(fx.url(path))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn usage_reports_sizes_and_404s_an_unknown_dataset() {
    let fx = fixture().await;
    let r = reqwest::get(fx.url(&format!("/datasets/{}/usage", fx.dataset_id)))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let u: Value = r.json().await.unwrap();
    assert_eq!(u["work_bytes"], 600);
    assert_eq!(u["work_files"], 3);
    assert_eq!(u["discarded_frames"], 2);
    assert_eq!(u["discarded_bytes"], 500);
    assert_eq!(u["export_dir"], Value::Null);
    assert_eq!(u["export_app_owned"], false);
    assert!(u["work_dir"]
        .as_str()
        .unwrap()
        .ends_with(fx.work_root.file_name().unwrap().to_string_lossy().as_ref()));

    let missing = reqwest::get(fx.url("/datasets/nope/usage")).await.unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn bulk_keep_clears_the_rejection_and_bulk_discard_excludes() {
    let fx = fixture().await;
    let path = format!("/datasets/{}/frames/bulk", fx.dataset_id);

    let (status, body) = post(
        &fx,
        &path,
        json!({ "frame_ids": [fx.frames[1], fx.frames[2], "unknown"], "excluded": false }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({ "requested": 3, "updated": 2 }));
    let blur = fx
        .app
        .db
        .dataset_frames()
        .get(&fx.frames[2])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blur.rejection_reason, "");
    assert!(!blur.excluded);

    let (status, body) = post(
        &fx,
        &path,
        json!({ "frame_ids": [fx.frames[0]], "excluded": true }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["updated"], 1);

    let (status, _) = post(
        &fx,
        "/datasets/nope/frames/bulk",
        json!({ "frame_ids": [], "excluded": true }),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn frames_delete_removes_files_and_reports_freed_bytes() {
    let fx = fixture().await;
    let (status, body) = post(
        &fx,
        &format!("/datasets/{}/frames/delete", fx.dataset_id),
        json!({ "frame_ids": [fx.frames[0], fx.frames[2]] }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["deleted"], 2);
    assert_eq!(body["deleted_files"], 2);
    assert_eq!(body["freed_bytes"], 400);
    assert_eq!(body["skipped_files"], json!([]));
    assert!(!fx.frame_files[0].exists());
    assert!(fx.frame_files[1].exists());
    assert!(!fx.frame_files[2].exists());

    let (status, _) = post(
        &fx,
        "/datasets/nope/frames/delete",
        json!({ "frame_ids": [] }),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn cleanup_previews_then_deletes_the_discarded_frames() {
    let fx = fixture().await;
    let path = format!("/datasets/{}/cleanup", fx.dataset_id);

    let (status, preview) = post(&fx, &path, json!({ "dry_run": true })).await;
    assert_eq!(status, 200);
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["frames"], 2);
    assert_eq!(preview["bytes"], 500);
    assert!(fx.frame_files.iter().all(|f| f.exists()));

    // A body without the flag is a preview too.
    let (_, implicit) = post(&fx, &path, json!({})).await;
    assert_eq!(implicit["dry_run"], true);
    assert!(fx.frame_files.iter().all(|f| f.exists()));

    let (status, done) = post(&fx, &path, json!({ "dry_run": false })).await;
    assert_eq!(status, 200);
    assert_eq!(done["dry_run"], false);
    assert_eq!(done["frames"], 2);
    assert_eq!(done["bytes"], 500);
    assert_eq!(done["deleted_files"], 2);
    assert!(fx.frame_files[0].exists());
    assert!(!fx.frame_files[1].exists());
    assert!(!fx.frame_files[2].exists());

    let (status, _) = post(&fx, "/datasets/nope/cleanup", json!({ "dry_run": true })).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn dedup_reports_its_summary_with_the_clamped_threshold() {
    let fx = fixture().await;
    let (status, body) = post(
        &fx,
        &format!("/datasets/{}/dedup", fx.dataset_id),
        json!({ "threshold": 99 }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["threshold"], 16);
    // The one kept frame is not a real PNG in this fixture.
    assert_eq!(body["scanned"], 1);
    assert_eq!(body["unreadable"], 1);
    assert_eq!(body["groups"], 0);
    assert_eq!(body["marked"], 0);

    let (status, _) = post(&fx, "/datasets/nope/dedup", json!({})).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn an_active_training_run_is_a_400_naming_the_run() {
    let fx = fixture().await;
    fx.app
        .db
        .training_runs()
        .create(NewTrainingRun {
            name: "Style v1".into(),
            profile_family: "flux2_klein_4b".into(),
            target_model_id: None,
            dataset_id: Some(fx.dataset_id.clone()),
            data_kind: DatasetMode::Frames,
            trigger_word: "demo_xy".into(),
            preset: Preset::Fast,
            hyperparams_json: "{}".into(),
            sample_prompts_json: "[]".into(),
            work_dir: "unused".into(),
            init_lora_model_id: None,
            image_count: None,
        })
        .await
        .unwrap();

    let del = reqwest::Client::new()
        .delete(fx.url(&format!("/datasets/{}", fx.dataset_id)))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 400);
    let err: Value = del.json().await.unwrap();
    assert!(err["error"].as_str().unwrap().contains("Style v1"), "{err}");

    let (status, _) = post(
        &fx,
        &format!("/datasets/{}/cleanup", fx.dataset_id),
        json!({ "dry_run": false }),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = post(
        &fx,
        &format!("/datasets/{}/frames/delete", fx.dataset_id),
        json!({ "frame_ids": [fx.frames[0]] }),
    )
    .await;
    assert_eq!(status, 400);
    assert!(fx.frame_files.iter().all(|f| f.exists()));
}

#[tokio::test]
async fn delete_dataset_removes_its_files_and_then_404s() {
    let fx = fixture().await;
    let http = reqwest::Client::new();
    let url = fx.url(&format!("/datasets/{}", fx.dataset_id));

    let r = http.delete(&url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["frames"], 3);
    assert_eq!(body["deleted_files"], 3);
    assert_eq!(body["freed_bytes"], 600);
    assert_eq!(body["skipped_files"], json!([]));
    assert_eq!(body["export_dir_kept"], Value::Null);
    assert_eq!(body["dataset_deleted"], true);
    assert!(!fx.work_root.exists());

    assert_eq!(http.get(&url).send().await.unwrap().status(), 404);
    assert_eq!(http.delete(&url).send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn more_than_ten_thousand_frame_ids_are_a_400() {
    let fx = fixture().await;
    let ids: Vec<String> = (0..10_001).map(|i| i.to_string()).collect();
    for route in ["frames/delete", "frames/bulk"] {
        let (status, body) = post(
            &fx,
            &format!("/datasets/{}/{route}", fx.dataset_id),
            json!({ "frame_ids": ids, "excluded": true }),
        )
        .await;
        assert_eq!(status, 400, "{route}: {body}");
    }
    assert!(fx.frame_files.iter().all(|f| f.exists()));
}

#[tokio::test]
async fn a_running_prep_job_is_a_400() {
    let fx = fixture().await;
    let job = fx
        .app
        .db
        .jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap();
    let busy = fx
        .app
        .db
        .datasets()
        .create(NewDataset {
            name: "Busy".into(),
            mode: DatasetMode::Frames,
            source_root: "E:\\Data\\Busy".into(),
            prep_job_id: Some(job.id),
            work_dir: None,
        })
        .await
        .unwrap();
    let r = reqwest::Client::new()
        .delete(fx.url(&format!("/datasets/{}", busy.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let err: Value = r.json().await.unwrap();
    assert!(
        err["error"]
            .as_str()
            .unwrap()
            .contains("still being prepared"),
        "{err}"
    );
}

/// A dataset prep job with a relative root is refused at submission (400),
/// before any job row exists.
#[tokio::test]
async fn a_relative_dataset_root_is_a_400_at_submission() {
    let fx = fixture().await;
    let before = fx
        .app
        .db
        .jobs()
        .list(&aiwm_core::db::JobFilter::default())
        .await
        .unwrap()
        .len();
    let (status, body) = post(
        &fx,
        "/jobs",
        json!({ "job_type": "dataset_prep", "params": { "root": "Data\\Ghibli" } }),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("absolute"),
        "{body}"
    );
    let after = fx
        .app
        .db
        .jobs()
        .list(&aiwm_core::db::JobFilter::default())
        .await
        .unwrap()
        .len();
    assert_eq!(before, after, "no job was created");
}
