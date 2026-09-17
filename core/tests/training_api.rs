//! The training HTTP surface driven end-to-end over a real loopback server.
//! No trainer is installed in a fresh temp store and none is installed here,
//! so this covers exactly the half the UI must survive without one: the
//! status card, the profile list, the form's own validation, the refusal a
//! run gets when the runtime is missing, and the run list / detail / delete
//! routes against a row put straight into the store.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use aiwm_core::db::{DatasetMode, NewTrainingRun, Preset, RunState};
use aiwm_core::{ApiServer, App, AppPaths};

/// A live server over a fresh store. The `TempDir` must outlive the test (it
/// backs the `App`'s data directory).
async fn fixture() -> (ApiServer, tempfile::TempDir, Arc<App>) {
    let tmp = tempfile::tempdir().unwrap();
    let app = Arc::new(App::load(AppPaths::rooted(tmp.path())).await.unwrap());
    let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    (server, tmp, app)
}

/// A `running`-looking row written straight through the repo — the poller and
/// the runner are not involved, which is the point: the read routes must work
/// off the stored row alone.
async fn insert_running_run(app: &App) -> String {
    let run = app
        .db
        .training_runs()
        .create(NewTrainingRun {
            name: "anime_style_v1".into(),
            profile_family: "flux2-klein-4b".into(),
            target_model_id: None,
            dataset_id: None,
            data_kind: DatasetMode::Frames,
            trigger_word: "ghibli_xy".into(),
            preset: Preset::Fast,
            hyperparams_json: "{}".into(),
            sample_prompts_json: "[\"ghibli_xy portrait\"]".into(),
            work_dir: "E:\\Data\\training\\does-not-exist".into(),
        })
        .await
        .unwrap();
    app.db
        .training_runs()
        .set_state(&run.id, RunState::Running)
        .await
        .unwrap();
    run.id
}

#[tokio::test]
async fn trainer_status_reports_a_missing_install() {
    let (server, _tmp, _app) = fixture().await;
    let resp = reqwest::get(format!("http://{}/training/status", server.addr))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["installed"], false);
    assert_eq!(status["installing"], false);
    assert_eq!(status["env_broken"], false);
    assert_eq!(status["install_state"]["state"], "idle");
    assert!(status["alive_run_id"].is_null());
    assert!(status["detail"].is_string(), "got: {status}");
}

#[tokio::test]
async fn installing_the_trainer_is_refused_in_offline_mode() {
    let (server, _tmp, app) = fixture().await;
    app.set_offline(true);

    let resp = reqwest::Client::new()
        .post(format!("http://{}/training/install", server.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("offline"),
        "got: {body}"
    );
}

#[tokio::test]
async fn profiles_list_with_their_base_weights_missing() {
    let (server, _tmp, _app) = fixture().await;
    let resp = reqwest::get(format!("http://{}/training/profiles", server.addr))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let profiles: serde_json::Value = resp.json().await.unwrap();
    let entries = profiles.as_array().unwrap();
    assert_eq!(entries.len(), 4, "the four seeded profiles");
    for p in entries {
        // A fresh store holds no model rows, so no base weights are staged.
        assert_eq!(p["base_installed"], false, "got: {p}");
        assert!(p["trainable_models"].as_array().unwrap().is_empty());
        assert!(p["fit_label"].is_string(), "got: {p}");
        assert!(p["presets"]["balanced"]["steps"].is_number(), "got: {p}");
    }
    let four_b = entries
        .iter()
        .find(|p| p["family"] == "flux2-klein-4b")
        .unwrap();
    assert_eq!(four_b["arch"], "flux2_klein_4b");
    assert_eq!(four_b["data_kind"], "frames");
    assert_eq!(four_b["fit"], "comfortable");
}

#[tokio::test]
async fn a_bad_trigger_word_is_rejected_before_anything_starts() {
    let (server, _tmp, _app) = fixture().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{}/training/runs", server.addr))
        .json(&serde_json::json!({
            "name": "Anime style v1",
            "target_model_id": "m-flux2",
            "dataset_id": "ds-1",
            "trigger_word": "has space here that is way too long for the limit",
            "preset": "fast",
            "hyperparams": {},
            "sample_prompts": ["ghibli_xy portrait"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("trigger word"),
        "got: {body}"
    );
}

#[tokio::test]
async fn starting_a_run_without_a_trainer_is_a_plain_400() {
    let (server, _tmp, _app) = fixture().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{}/training/runs", server.addr))
        .json(&serde_json::json!({
            "name": "Anime style v1",
            "target_model_id": "m-flux2",
            "dataset_id": "ds-1",
            "trigger_word": "ghibli_xy",
            "preset": "fast",
            "hyperparams": {},
            "sample_prompts": ["ghibli_xy portrait"],
        }))
        .send()
        .await
        .unwrap();
    // A missing runtime is the user's to fix, not a server fault: 400 with a
    // sentence the Training tab can show as-is.
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("not installed") || message.contains("set it up"),
        "got: {body}"
    );
}

#[tokio::test]
async fn runs_are_listed_read_and_only_deletable_once_terminal() {
    let (server, _tmp, app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();
    let run_id = insert_running_run(&app).await;

    let listed: serde_json::Value = http
        .get(format!("{base}/training/runs"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], run_id.as_str());
    assert_eq!(listed[0]["state"], "running");
    assert_eq!(listed[0]["preset"], "fast");

    let detail_resp = http
        .get(format!("{base}/training/runs/{run_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(detail_resp.status(), 200);
    let detail: serde_json::Value = detail_resp.json().await.unwrap();
    assert_eq!(detail["run"]["id"], run_id.as_str());
    // Nothing has been written to disk, so there is no log and no sample --
    // both empty, neither an error.
    assert_eq!(detail["log_tail"], serde_json::json!([]));
    assert_eq!(detail["latest_samples"], serde_json::json!([]));
    assert!(detail["work_dir"].is_string(), "got: {detail}");

    // A run still going is not deletable -- cancel it first.
    let too_early = http
        .delete(format!("{base}/training/runs/{run_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(too_early.status(), 400);
    let body: serde_json::Value = too_early.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("still"),
        "got: {body}"
    );

    app.db
        .training_runs()
        .set_state(&run_id, RunState::Cancelled)
        .await
        .unwrap();

    let deleted = http
        .delete(format!("{base}/training/runs/{run_id}?purge=false"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);

    let listed: serde_json::Value = http
        .get(format!("{base}/training/runs"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed, serde_json::json!([]));
}

#[tokio::test]
async fn an_unknown_run_is_404_everywhere() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();

    let missing = http
        .get(format!("{base}/training/runs/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let body: serde_json::Value = missing.json().await.unwrap();
    assert_eq!(body["error"], "no such training run");

    let sample = http
        .get(format!("{base}/training/runs/nope/samples/0"))
        .send()
        .await
        .unwrap();
    assert_eq!(sample.status(), 404);
}
