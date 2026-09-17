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
use aiwm_core::{ApiServer, App, AppOptions, AppPaths};

/// A live server over a fresh store. The `TempDir` must outlive the test (it
/// backs the `App`'s data directory).
///
/// The training poller is off. These tests write `running` rows by hand and
/// then assert on them; the live poller re-reads every such row every few
/// seconds, finds no process behind it, and reconciles it to `interrupted` —
/// correctly, but racing the assertions. Startup recovery still runs (it only
/// ever sees rows from a previous process, and the store is empty here).
/// What is under test is the HTTP surface; the poller has its own tests in
/// `core::training::runner`.
async fn fixture() -> (ApiServer, tempfile::TempDir, Arc<App>) {
    let tmp = tempfile::tempdir().unwrap();
    let app = Arc::new(
        App::load_with(
            AppPaths::rooted(tmp.path()),
            AppOptions {
                training_poller: false,
            },
        )
        .await
        .unwrap(),
    );
    let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    (server, tmp, app)
}

fn new_run() -> NewTrainingRun {
    NewTrainingRun {
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
    }
}

/// A `running`-looking row written straight through the repo — the poller and
/// the runner are not involved, which is the point: the read routes must work
/// off the stored row alone.
async fn insert_running_run(app: &App) -> String {
    let run = app.db.training_runs().create(new_run()).await.unwrap();
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
async fn starting_and_resuming_a_run_are_refused_in_offline_mode() {
    // ADR-009. Staged base weights are not enough to train offline: on a
    // family's first run ai-toolkit fetches the Qwen3 text encoder and the
    // FLUX.2 VAE from the Hub itself (confirmed on the real 4B run, which
    // pulled 8 GB of Qwen3-4B after the local blob had already loaded). A
    // run started with the network switched off would get minutes in and
    // then die on a download, so it is refused up front.
    let (server, _tmp, app) = fixture().await;
    let run_id = insert_running_run(&app).await;
    app.set_offline(true);
    let client = reqwest::Client::new();

    let start = client
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
    assert_eq!(start.status(), 400);
    let body: serde_json::Value = start.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("offline"),
        "got: {body}"
    );

    let resume = client
        .post(format!(
            "http://{}/training/runs/{run_id}/resume",
            server.addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resume.status(), 400);
    let body: serde_json::Value = resume.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("offline"),
        "got: {body}"
    );
}

#[tokio::test]
async fn probing_without_a_trainer_is_a_refusal_not_a_fault() {
    let (server, _tmp, _app) = fixture().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{}/training/probe", server.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("not installed") || message.contains("set it up"),
        "got: {body}"
    );
}

/// The counterweight to every "a refusal is a 400" assertion above: a fault
/// the user cannot act on must stay a 500. `hyperparams_json` is written by
/// us and read back by us, so a row whose copy is unreadable is a corrupt
/// store, not a mistake anyone made in the form.
#[tokio::test]
async fn an_unreadable_stored_row_is_a_fault_not_a_refusal() {
    let (server, _tmp, app) = fixture().await;
    let run = app
        .db
        .training_runs()
        .create(NewTrainingRun {
            hyperparams_json: "{ this is not json".into(),
            ..new_run()
        })
        .await
        .unwrap();
    let runs = app.db.training_runs();
    runs.set_state(&run.id, RunState::Running).await.unwrap();
    runs.set_state(&run.id, RunState::Interrupted)
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!(
            "http://{}/training/runs/{}/resume",
            server.addr, run.id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500, "a corrupt row is not the user's fault");
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
async fn pause_resume_and_cancel_move_a_run_through_its_lifecycle() {
    let (server, _tmp, app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();
    let run_id = insert_running_run(&app).await;

    // Pausing a run whose process is already gone cannot report a pause that
    // never happened: the runner reconciles it from the log and checkpoints
    // instead, and a silent disappearance is `interrupted`, never `failed`.
    let paused = http
        .post(format!("{base}/training/runs/{run_id}/pause"))
        .send()
        .await
        .unwrap();
    assert_eq!(paused.status(), 200);
    let run: serde_json::Value = paused.json().await.unwrap();
    assert_eq!(run["state"], "interrupted", "got: {run}");

    // Continuing it needs a trainer, and there is none in a fresh store --
    // the refusal is the user's to act on, so 400 with a readable sentence.
    let resumed = http
        .post(format!("{base}/training/runs/{run_id}/resume"))
        .send()
        .await
        .unwrap();
    assert_eq!(resumed.status(), 400);
    let body: serde_json::Value = resumed.json().await.unwrap();
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("not installed") || message.contains("set it up"),
        "got: {body}"
    );

    // A failed resume leaves the run exactly where it was.
    let detail: serde_json::Value = http
        .get(format!("{base}/training/runs/{run_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["run"]["state"], "interrupted");

    let cancelled = http
        .post(format!("{base}/training/runs/{run_id}/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), 200);
    let run: serde_json::Value = cancelled.json().await.unwrap();
    assert_eq!(run["state"], "cancelled", "got: {run}");

    // And a second pause is a refusal, not a 500: the run is terminal now.
    let again = http
        .post(format!("{base}/training/runs/{run_id}/pause"))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 400);
    let body: serde_json::Value = again.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("only a running"),
        "got: {body}"
    );
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
