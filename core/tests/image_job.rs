//! End-to-end: a `job_type=image` job through the real `JobEngine` — scheduler
//! decision, `ComfyUiAdapter` starts the (fake) server, the image body posts the
//! workflow, polls `/history`, fetches the image via `/view`, writes it to the
//! outputs dir, the job reaches `Completed` with `output_path` set.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ComfyDirs, ComfyLaunch, ComfyUiAdapter, LlamaCppAdapter, RuntimeAdapter, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;

/// `detail()` always renders the port right after `verb :` (`format!("{verb}
/// :{port}")`, see `comfyui_adapter.rs`'s own copy of this helper) — pull it
/// back out so a test can hit the fake server's test-only endpoints directly.
fn port_from_detail(detail: &str) -> u16 {
    let after_colon = detail
        .split_once(':')
        .expect("detail should contain a port")
        .1;
    after_colon
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("port digits")
}

fn fake_comfy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-comfy"))
}

fn image_job(prompt: &str) -> NewJob {
    let mut job = NewJob::new("image");
    job.params = serde_json::json!({ "prompt": prompt, "width": 512, "height": 512, "steps": 4 });
    job
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    outputs: PathBuf,
    /// Kept so a test can reach the fake ComfyUI's test-only endpoints
    /// directly (`detail()` names the port once the server is up).
    comfyui: Arc<ComfyUiAdapter>,
    _tmp: tempfile::TempDir,
}

async fn harness(with_model: bool) -> Harness {
    harness_with(with_model, &[]).await
}

/// Engine wired to the fake ComfyUI. `with_model` registers a `base_diffusion`
/// checkpoint; `extra_args` are appended to the fixture's command line.
async fn harness_with(with_model: bool, extra_args: &[&str]) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let outputs = tmp.path().join("outputs");

    if with_model {
        let ckpt = tmp.path().join("sd_xl_base_1.0.safetensors");
        std::fs::write(&ckpt, b"safetensors fixture").unwrap();
        db.models()
            .insert(NewModel {
                name: "SDXL Base".into(),
                family: Some("sdxl".into()),
                format: "safetensors".into(),
                file_path: ckpt.to_string_lossy().into_owned(),
                size_bytes: 6_000 * 1024 * 1024,
                vram_estimate_mb: Some(8_000),
                source: "manual".into(),
                roles: vec!["base_diffusion".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
    }

    let registry = RuntimeRegistry::new();
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    registry.register(llama.clone());
    let comfyui = Arc::new(ComfyUiAdapter::with_launch(
        db.clone(),
        Some(ComfyLaunch {
            program: fake_comfy_bin(),
            main: None,
            extra_args: extra_args.iter().map(|s| (*s).to_string()).collect(),
        }),
        ComfyDirs {
            base: tmp.path().join("comfyui-data"),
            output: outputs.clone(),
            models_store: tmp.path().join("store"),
        },
    ));
    registry.register(comfyui.clone());

    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
    let engine = Arc::new(JobEngine::new(
        db.clone(),
        registry,
        scheduler,
        llama,
        comfyui.clone(),
        outputs.clone(),
    ));

    Harness {
        db,
        engine,
        outputs,
        comfyui,
        _tmp: tmp,
    }
}

impl Harness {
    /// Register a model file with a role (companion models for Flux tests).
    async fn add_model(&self, name: &str, family: Option<&str>, role: &str) -> String {
        let path = self._tmp.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        self.db
            .models()
            .insert(NewModel {
                name: name.into(),
                family: family.map(str::to_string),
                format: "safetensors".into(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: 1_000,
                vram_estimate_mb: Some(9_000),
                source: "manual".into(),
                roles: vec![role.into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }
}

fn is_png(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.starts_with(&[0x89, 0x50, 0x4e, 0x47]))
        .unwrap_or(false)
}

#[tokio::test]
async fn auto_image_job_renders_and_writes_the_file() {
    let h = harness(true).await;
    let job = h
        .engine
        .submit(image_job("a red fox in the snow"))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert_eq!(stored.runtime_id.as_deref(), Some("comfyui"));
    assert!(
        stored.model_id.is_some(),
        "Auto should have bound a checkpoint"
    );
    assert!(stored.finished_at.is_some());

    let out = stored.output_path.expect("output_path set");
    assert_eq!(out, h.outputs.join(format!("{}.png", job.id)));
    assert!(is_png(Path::new(&out)), "a real PNG was written");

    // The random seed was pinned back onto the job.
    let seed = stored.params["seed"].as_i64().expect("seed persisted");
    assert!(seed >= 0);

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events.iter().any(|m| m.contains("auto-selected")),
        "{events:?}"
    );
    assert!(
        events.iter().any(|m| m.contains("image ready")),
        "{events:?}"
    );

    assert_eq!(h.db.models().list().await.unwrap()[0].use_count, 1);
}

#[tokio::test]
async fn explicit_checkpoint_image_job_completes() {
    let h = harness(true).await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();

    let job = h
        .engine
        .submit(image_job("Ping").on("comfyui", &model_id, 8_000))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.output_path.is_some());
}

/// The UI's new LoRA-stack picker (`ui/src/components/LoraPicker.tsx`) submits
/// `params.loras = [{ model_id, strength }, …]`. This proves that shape makes
/// it all the way through the real job pipeline — `ImageRequest::from_params`
/// (`parse_loras`) → `resolve_loras` (library lookup) →
/// `pipeline::checkpoint_txt2img`'s `loras: &[LoraSpec]` — by inspecting the
/// *actual graph ComfyUI received* (via the fake server's
/// `/__test/last_lora_chain`), not just that the job completed. Mirrors the
/// unit-level proof already covering each link (`parse_loras_reads_ids_and_
/// clamps_strength`, `resolve_loras_resolves_the_bare_file_name`,
/// `checkpoint_graph_splices_a_lora_before_the_sampler_and_clip`) with one
/// end-to-end check that they actually compose.
#[tokio::test]
async fn an_image_job_with_a_lora_splices_a_loraloader_into_the_real_graph() {
    let h = harness(true).await;
    let lora_path = h._tmp.path().join("add-detail-xl.safetensors");
    std::fs::write(&lora_path, b"fixture").unwrap();
    let lora =
        h.db.models()
            .insert(NewModel {
                name: "Add Detail XL".into(),
                family: Some("sdxl".into()),
                format: "safetensors".into(),
                file_path: lora_path.to_string_lossy().into_owned(),
                size_bytes: 1_000,
                source: "manual".into(),
                roles: vec!["lora".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();

    let mut job = image_job("a red fox in the snow");
    job.params["loras"] = serde_json::json!([{ "model_id": lora.id, "strength": 0.65 }]);
    let job = h.engine.submit(job).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    // The resolved LoRA is pinned back onto the job's own params too (3.5's
    // gallery needs concrete values, same as seed/width/height).
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.params["loras"][0]["model_id"], lora.id.as_str());
    assert_eq!(stored.params["loras"][0]["strength"], 0.65);

    // The real proof: ask the fake ComfyUI what graph it actually received.
    let port = port_from_detail(&h.comfyui.detail().expect("comfyui detail"));
    let resp: serde_json::Value =
        reqwest::get(format!("http://127.0.0.1:{port}/__test/last_lora_chain"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    let chain = resp["loras"].as_array().expect("loras array");
    assert_eq!(chain.len(), 1, "{resp:?}");
    assert_eq!(chain[0]["file"], "add-detail-xl.safetensors");
    assert_eq!(chain[0]["strength"], 0.65);
}

/// Plan 3's Hi-Res-Fix, proved the same way the LoRA case above is: by asking
/// the fake ComfyUI what graph it actually received. `params.hires` →
/// `ImageRequest::from_params` → `Txt2ImgInputs.hires` →
/// `fragments::hires::ksampler_pass` has to come out the far end as a real
/// `LatentUpscaleBy` plus a *second* `KSampler`. The no-hires control runs on
/// the same fixture first so the difference — not just the presence of the
/// nodes — is what's being asserted.
#[tokio::test]
async fn an_image_job_with_hires_fix_submits_a_second_sampler_pass() {
    let h = harness(true).await;

    // --- control: the same job without `hires` ---------------------------
    let plain = h.engine.submit(image_job("a quiet harbour")).await.unwrap();
    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == plain.id),
        "got {outcome:?}"
    );
    let port = port_from_detail(&h.comfyui.detail().expect("comfyui detail"));
    let types = graph_node_types(port).await;
    assert_eq!(count(&types, "KSampler"), 1, "{types:?}");
    assert_eq!(count(&types, "LatentUpscaleBy"), 0, "{types:?}");

    // --- the Hi-Res-Fix job ----------------------------------------------
    let mut job = image_job("a quiet harbour, high detail");
    job.params["steps"] = serde_json::json!(20);
    job.params["hires"] = serde_json::json!({ "scale_by": 1.5, "denoise": 0.45 });
    let job = h.engine.submit(job).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let types = graph_node_types(port).await;
    assert_eq!(count(&types, "LatentUpscaleBy"), 1, "{types:?}");
    assert_eq!(count(&types, "KSampler"), 2, "{types:?}");

    // The resolved request is pinned back onto the job: the second pass's
    // steps defaulted to half the first pass's, and the finished pixel size
    // is the upscaled one (512 px = 64 latent units, × 1.5 → 96 → 768 px).
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.params["hires"]["steps"], 10);
    assert_eq!(stored.params["hires"]["scale_by"], 1.5);
    assert_eq!(stored.params["hires"]["upscale_method"], "nearest-exact");
    assert_eq!(stored.params["output_width"], 768);
    assert_eq!(stored.params["output_height"], 768);
}

/// The `class_type`s of the last graph the fake ComfyUI received, in node-id
/// order (its `/__test/last_graph_node_types`).
async fn graph_node_types(port: u16) -> Vec<String> {
    let resp: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{port}/__test/last_graph_node_types"
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    resp.as_array()
        .expect("node type array")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect()
}

fn count(types: &[String], class_type: &str) -> usize {
    types.iter().filter(|t| *t == class_type).count()
}

#[tokio::test]
async fn image_job_without_a_prompt_fails() {
    let h = harness(true).await;
    let job = h.engine.submit(NewJob::new("image")).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Failed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.error_text.unwrap_or_default().contains("prompt"));
}

#[tokio::test]
async fn auto_image_job_fails_cleanly_without_an_image_model() {
    let h = harness(false).await;
    h.engine.submit(image_job("hi")).await.unwrap();

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => {
            assert!(error.contains("no image model"), "{error}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_comfyui_execution_error_fails_the_job() {
    let h = harness_with(true, &["--fake-history-error"]).await;
    let job = h.engine.submit(image_job("boom")).await.unwrap();

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("fake render error"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Failed);
}

#[tokio::test]
async fn a_flux_job_without_its_companion_models_fails_with_a_clear_message() {
    let h = harness(false).await;
    h.add_model("flux1-dev-Q8_0.gguf", Some("flux"), "base_diffusion")
        .await;

    let job = h.engine.submit(image_job("a flux render")).await.unwrap();
    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => {
            assert!(
                error.contains("T5") && error.contains("Models tab"),
                "{error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(h
        .db
        .jobs()
        .get(&job.id)
        .await
        .unwrap()
        .unwrap()
        .output_path
        .is_none());
}

#[tokio::test]
async fn a_flux_job_with_all_companions_completes() {
    let h = harness(false).await;
    h.add_model("flux1-dev-Q8_0.gguf", Some("flux"), "base_diffusion")
        .await;
    h.add_model("t5xxl_fp8_e4m3fn.safetensors", None, "text_encoder")
        .await;
    h.add_model("clip_l.safetensors", None, "text_encoder")
        .await;
    h.add_model("ae.safetensors", Some("flux"), "vae").await;

    let job = h
        .engine
        .submit(image_job("a fox, flux style"))
        .await
        .unwrap();
    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(is_png(Path::new(stored.output_path.as_ref().unwrap())));

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events
            .iter()
            .any(|m| m.contains("Flux") && m.contains("T5")),
        "{events:?}"
    );
}

#[tokio::test]
async fn cancel_stops_a_running_image_job_mid_render() {
    // /history reports "pending" for 4 s — plenty of time to cancel.
    let h = harness_with(true, &["--fake-render-ms", "4000"]).await;
    let job = h.engine.submit(image_job("a long render")).await.unwrap();

    let engine = h.engine.clone();
    let run = tokio::spawn(async move { engine.run_next().await.unwrap().unwrap() });

    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        h.engine.cancel(&job.id).await.unwrap(),
        "cancel should apply"
    );

    let outcome = run.await.unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Cancelled { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Cancelled);
    assert!(stored.finished_at.is_some());
    assert!(stored.output_path.is_none(), "no image on a cancel");

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events
            .iter()
            .any(|m| m.contains("cancelled while rendering")),
        "{events:?}"
    );
}
