//! The dataset/concept HTTP surface driven end-to-end over a real loopback
//! server: status codes, the three-valued clip-bound body, and the
//! `requested`/`attached` split an "Alle im Set" click reports. No sidecar and
//! no llama.cpp are involved — these routes only touch the store.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use aiwm_core::db::{DatasetMode, NewDataset, NewDatasetFrame, NewJob};
use aiwm_core::{ApiServer, App, AppPaths};

/// A live server plus one dataset and two frames in it. The `TempDir` must
/// outlive the test (it backs the `App`'s data directory).
async fn fixture() -> (ApiServer, tempfile::TempDir, String, Vec<String>) {
    let tmp = tempfile::tempdir().unwrap();
    let app = Arc::new(App::load(AppPaths::rooted(tmp.path())).await.unwrap());

    let job = app
        .db
        .jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap();
    let dataset = app
        .db
        .datasets()
        .create(NewDataset {
            name: "Demo".into(),
            mode: DatasetMode::Clips,
            source_root: "E:\\Data\\Demo".into(),
            prep_job_id: Some(job.id.clone()),
            work_dir: None,
        })
        .await
        .unwrap();

    let mut frames = Vec::new();
    for i in 0..2 {
        let f = app
            .db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job.id.clone(),
                dataset_id: Some(dataset.id.clone()),
                tag: "Ghibli".into(),
                source_path: "clip.mp4".into(),
                frame_path: format!("clip_{i}.png"),
                timestamp_secs: Some(f64::from(i)),
                rejection_reason: String::new(),
                duration_secs: Some(6.0),
            })
            .await
            .unwrap();
        frames.push(f.id);
    }

    let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    (server, tmp, dataset.id, frames)
}

#[tokio::test]
async fn concepts_are_created_assigned_and_deleted_over_http() {
    let (server, _tmp, dataset_id, frames) = fixture().await;
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();

    // 201 with the stored row -- the caller never has to guess the new id.
    let created = http
        .post(format!("{base}/datasets/{dataset_id}/concepts"))
        .json(&serde_json::json!({ "name": "Kenji", "token": "kenji_xy" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let concept: serde_json::Value = created.json().await.unwrap();
    let concept_id = concept["id"].as_str().unwrap().to_string();
    assert_eq!(concept["token"], "kenji_xy");

    // A token already taken in this dataset is a user mistake -> 400, not the
    // 500 a raw UNIQUE violation would produce.
    let dup = http
        .post(format!("{base}/datasets/{dataset_id}/concepts"))
        .json(&serde_json::json!({ "name": "Other", "token": "kenji_xy" }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 400);

    let assign = |ids: Vec<String>| {
        let http = http.clone();
        let url = format!("{base}/concepts/{concept_id}/frames");
        async move {
            let r = http
                .post(url)
                .json(&serde_json::json!({ "frame_ids": ids }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            r.json::<serde_json::Value>().await.unwrap()
        }
    };

    let first = assign(frames.clone()).await;
    assert_eq!(first["requested"], 2);
    assert_eq!(first["attached"], 2);
    // Idempotent: the second click attaches nothing new.
    let again = assign(frames.clone()).await;
    assert_eq!(again["requested"], 2);
    assert_eq!(again["attached"], 0);

    let map: serde_json::Value = http
        .get(format!("{base}/datasets/{dataset_id}/frame-concepts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for f in &frames {
        assert_eq!(map[f][0], serde_json::Value::String(concept_id.clone()));
    }

    let summaries: serde_json::Value = http
        .get(format!("{base}/datasets/{dataset_id}/concepts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summaries[0]["frame_count"], 2);
    assert!(summaries[0]["token_warning"].is_null(), "kenji_xy is fine");

    // A concept named after an ordinary word carries the inline warning.
    let plain = http
        .post(format!("{base}/datasets/{dataset_id}/concepts"))
        .json(&serde_json::json!({ "name": "Anime look", "token": "anime" }))
        .send()
        .await
        .unwrap();
    assert_eq!(plain.status(), 201);
    let summaries: serde_json::Value = http
        .get(format!("{base}/datasets/{dataset_id}/concepts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let warned = summaries
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["token"] == "anime")
        .unwrap();
    assert!(
        warned["token_warning"].as_str().unwrap().contains("anime"),
        "got: {warned}"
    );

    // Deleting a concept takes its assignments with it.
    let deleted = http
        .delete(format!("{base}/concepts/{concept_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    let map: serde_json::Value = http
        .get(format!("{base}/datasets/{dataset_id}/frame-concepts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(map, serde_json::json!({}));
}

#[tokio::test]
async fn an_explicit_null_clip_bound_clears_only_that_bound() {
    let (server, _tmp, dataset_id, frames) = fixture().await;
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();

    // The job-keyed edit route: the frame id is what identifies the row, so
    // any job segment routes to the same handler.
    let edit_url = format!("{base}/jobs/any/dataset-frames/{}", frames[0]);
    let set = http
        .put(&edit_url)
        .json(&serde_json::json!({ "clip_start_secs": 1.5, "clip_end_secs": 4.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), 200);

    let cleared = http
        .put(&edit_url)
        .json(&serde_json::json!({ "clip_end_secs": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 200);

    let listed: serde_json::Value = http
        .get(format!("{base}/datasets/{dataset_id}/frames"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let frame = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == frames[0].as_str())
        .unwrap();
    assert_eq!(frame["clip_start_secs"], 1.5, "in-point survives");
    assert!(frame["clip_end_secs"].is_null(), "out-point cleared");
}

#[tokio::test]
async fn a_dataset_is_fetchable_by_id_and_missing_ones_are_404() {
    let (server, _tmp, dataset_id, _frames) = fixture().await;
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();

    let found = http
        .get(format!("{base}/datasets/{dataset_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(found.status(), 200);
    let dataset: serde_json::Value = found.json().await.unwrap();
    assert_eq!(dataset["mode"], "clips");
    assert_eq!(dataset["name"], "Demo");

    let missing = http
        .get(format!("{base}/datasets/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let body: serde_json::Value = missing.json().await.unwrap();
    assert_eq!(body["error"], "no such dataset");
}

#[tokio::test]
async fn captioners_list_with_their_install_state() {
    let (server, _tmp, _dataset_id, _frames) = fixture().await;
    let resp = reqwest::get(format!("http://{}/captioners", server.addr))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let captioners: serde_json::Value = resp.json().await.unwrap();
    let entries = captioners.as_array().unwrap();
    assert_eq!(entries.len(), 2, "Florence-2 and the WD tagger");
    // A fresh store has no model rows, so neither captioner is usable yet.
    for c in entries {
        assert_eq!(c["installed"], false, "got: {c}");
        assert!(c["id"].is_string() && c["style"].is_string(), "got: {c}");
    }
}
