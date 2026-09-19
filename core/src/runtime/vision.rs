//! Vision-language-model runtime adapter — a thin [`RuntimeAdapter`] wrapper
//! around the Python sidecar's `caption_frame` / `caption_frame_pair`
//! (Florence-2 / Qwen2.5-VL — see `capability::dataset::caption`), so
//! `job_type=dataset_prep` jobs fit the same Target/scheduler pipeline every
//! other job goes through.
//!
//! Unlike [`super::TtsAdapter`] (Kokoro: a tiny CPU model that never charges
//! against the VRAM budget), Florence-2 and Qwen2.5-VL are real GPU-resident
//! models — a few hundred MB to several GB each. This adapter tracks what it
//! actually has loaded in [`loaded_models`](RuntimeAdapter::loaded_models)
//! rather than always reporting empty, so the scheduler's eviction logic
//! (`registry.runtime_with_model` \u{2192} `unload_model`) can reclaim that
//! VRAM for another job exactly the way it already does for llama.cpp/
//! ComfyUI models — see `capability::dataset::caption` for the fallback VRAM
//! numbers used before a job has actually loaded anything. `unload_model`
//! also tells the sidecar to drop its cached engines, and the job engine
//! calls it whenever a `dataset_prep` job ends (`JobEngine::drive`).

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::OnceCell;

use super::{Health, LoadedModel, RuntimeAdapter, RuntimeKind, SpawnSpec};
use crate::sidecar::{SidecarClient, SidecarSpec};
use crate::Result;

/// Adapter id — also the `runtimes` table key and the registry key.
pub const RUNTIME_ID: &str = "vision";

/// Sidecar JSON-RPC method that drops every cached captioner engine and
/// empties the CUDA cache (`aiwm_sidecar.vision.unload_vision_models`).
const UNLOAD_METHOD: &str = "unload_vision_models";

/// How long an unload may take before the core gives up waiting. Dropping
/// the engines and emptying the CUDA cache takes well under a second
/// (measured 2026-09-19); a sidecar that is stuck must not stall the job
/// queue, since the engine awaits the release after every dataset prep.
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Construction never fails, same reasoning as `TtsAdapter::new` — resolving
/// `uv` and spawning the sidecar is deferred to first real use.
#[derive(Debug)]
pub struct VisionAdapter {
    spec_override: Option<SidecarSpec>,
    sidecar: OnceCell<SidecarClient>,
    /// What this adapter currently considers resident — real accounting,
    /// updated by [`Self::load_model`] / [`Self::unload_model`], not a
    /// permanently-empty stand-in the way `TtsAdapter`'s is.
    loaded: Mutex<Vec<LoadedModel>>,
    /// [`UNLOAD_TIMEOUT`]; shorter only in tests.
    unload_timeout: Duration,
}

impl Default for VisionAdapter {
    fn default() -> Self {
        Self {
            spec_override: None,
            sidecar: OnceCell::new(),
            loaded: Mutex::new(Vec::new()),
            unload_timeout: UNLOAD_TIMEOUT,
        }
    }
}

impl VisionAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_spec(spec: SidecarSpec) -> Self {
        Self {
            spec_override: Some(spec),
            ..Self::default()
        }
    }

    /// Ensure the sidecar process is up, spawning it on first use — every
    /// caller shares the one instance, same as `TtsAdapter::client`.
    pub async fn client(&self) -> Result<&SidecarClient> {
        self.sidecar
            .get_or_try_init(|| async {
                let spec = match &self.spec_override {
                    Some(s) => s.clone(),
                    None => crate::sidecar::dev_spec()?,
                };
                SidecarClient::spawn(spec).await
            })
            .await
    }

    fn loaded_mut(&self) -> std::sync::MutexGuard<'_, Vec<LoadedModel>> {
        self.loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl RuntimeAdapter for VisionAdapter {
    fn id(&self) -> &str {
        RUNTIME_ID
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Vision
    }

    /// No process-supervisor spawn — same reasoning as `TtsAdapter`: the
    /// sidecar manages its own lifecycle via `SidecarClient`.
    fn spawn_spec(&self) -> Option<SpawnSpec> {
        None
    }

    async fn health(&self) -> Health {
        if self.sidecar.initialized() {
            Health::Healthy
        } else {
            Health::Unknown
        }
    }

    /// Ensures the sidecar is up and records `model_id` as resident with
    /// `vram_mb` charged against the budget — replacing any previous entry
    /// for the same id (a re-load with a different estimate updates the
    /// number rather than double-counting it).
    async fn load_model(&self, model_id: &str, vram_mb: u64) -> Result<()> {
        self.client().await?;
        let mut loaded = self.loaded_mut();
        loaded.retain(|m| m.model_id != model_id);
        loaded.push(LoadedModel {
            model_id: model_id.to_string(),
            vram_mb,
        });
        Ok(())
    }

    /// Drops `model_id` from the resident set and asks the sidecar to release
    /// its cached captioner engines (`unload_vision_models`: drop the
    /// references, `gc.collect()`, `torch.cuda.empty_cache()`), so the VRAM
    /// really goes back — measured 2026-09-19, Florence-2 and Qwen2.5-VL
    /// otherwise stayed resident until the daemon stopped.
    ///
    /// The bookkeeping is cleared first and a sidecar error is only logged:
    /// the ledger must never keep claiming a model the core asked to drop,
    /// and an eviction must not fail the job that needed the room. A sidecar
    /// that was never started holds nothing and is not spawned just for this.
    /// The synthetic pipeline id covers every captioner, so the release is
    /// not narrowed to one model.
    async fn unload_model(&self, model_id: &str) -> Result<()> {
        self.loaded_mut().retain(|m| m.model_id != model_id);
        let Some(client) = self.sidecar.get() else {
            return Ok(());
        };
        // Bounded: the engine awaits this after every dataset prep, so a stuck
        // sidecar must not stall the job queue. A reply that arrives after
        // the deadline is skipped by the next call (`raw_call` matches ids).
        let call = client.call(UNLOAD_METHOD, serde_json::json!({}));
        match tokio::time::timeout(self.unload_timeout, call).await {
            Ok(Ok(report)) => {
                tracing::info!(%model_id, %report, "vision sidecar released its captioners");
            }
            Ok(Err(e)) => {
                tracing::warn!(%model_id, error = %e, "vision sidecar could not release its captioners");
            }
            Err(_) => {
                tracing::warn!(
                    %model_id,
                    timeout_secs = self.unload_timeout.as_secs_f64(),
                    "vision sidecar did not answer the unload in time; its VRAM may still be held"
                );
            }
        }
        Ok(())
    }

    fn loaded_models(&self) -> Vec<LoadedModel> {
        self.loaded_mut().clone()
    }

    fn detail(&self) -> Option<String> {
        let loaded = self.loaded_mut();
        Some(if loaded.is_empty() {
            "not started yet".to_string()
        } else {
            let names = loaded
                .iter()
                .map(|m| m.model_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("loaded: {names}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_model_charges_real_vram_unlike_tts() {
        let adapter = VisionAdapter::with_spec(SidecarSpec {
            program: "definitely-not-a-real-binary".into(),
            args: vec![],
            env: vec![],
        });
        assert_eq!(adapter.id(), "vision");
        assert_eq!(adapter.kind(), RuntimeKind::Vision);
        assert!(adapter.spawn_spec().is_none());
        assert_eq!(adapter.vram_used_mb(), 0);

        // The spawn fails (no such binary) -- load_model must surface that
        // error rather than silently pretending the model loaded.
        assert!(adapter.load_model("florence2", 2048).await.is_err());
        assert_eq!(adapter.vram_used_mb(), 0);
        assert!(adapter.loaded_models().is_empty());
    }

    #[test]
    fn construction_never_fails_even_without_uv_on_path() {
        let adapter = VisionAdapter::new();
        assert_eq!(adapter.detail().as_deref(), Some("not started yet"));
    }

    /// A stand-in sidecar: a few lines of Python (run with the sidecar
    /// project's interpreter) that answer the handshake, append every other
    /// method name to `AIWM_FAKE_SIDECAR_LOG`, and answer
    /// `unload_vision_models` per `AIWM_FAKE_SIDECAR_UNLOAD`: `ok` (a
    /// report), `fail` (an RPC error) or `hang` (no answer for 20 s).
    const FAKE_SIDECAR: &str = r#"
import json, os, sys, time
log = os.environ["AIWM_FAKE_SIDECAR_LOG"]
mode = os.environ.get("AIWM_FAKE_SIDECAR_UNLOAD", "ok")
for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    if method == "handshake":
        out = {"result": {"sidecar_version": "fake", "protocol_version": 1, "capabilities": []}}
    else:
        with open(log, "a", encoding="utf-8") as f:
            f.write(method + "\n")
        if method == "unload_vision_models" and mode == "hang":
            time.sleep(20)
        if method == "unload_vision_models" and mode != "fail":
            out = {"result": {"released": [{"kind": "florence2", "model_dir": "x"}], "cuda_cache_cleared": True}}
        else:
            out = {"error": {"code": -32000, "message": "simulated failure"}}
    out.update(jsonrpc="2.0", id=req.get("id"))
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

    /// `None` (test skipped loudly) where `uv` is missing, like the sidecar
    /// client's own tests. `unload` is the fake's `AIWM_FAKE_SIDECAR_UNLOAD`.
    fn fake_sidecar_spec(log: &std::path::Path, unload: &str) -> Option<SidecarSpec> {
        let Some(uv) = crate::sidecar::resolve_uv() else {
            eprintln!("skipping fake-sidecar test: `uv` was not found");
            return None;
        };
        let sidecar_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("sidecar");
        let env = vec![
            ("PYTHONUNBUFFERED".to_string(), "1".to_string()),
            (
                "AIWM_FAKE_SIDECAR_LOG".to_string(),
                log.to_string_lossy().into_owned(),
            ),
            ("AIWM_FAKE_SIDECAR_UNLOAD".to_string(), unload.to_string()),
        ];
        Some(SidecarSpec {
            program: uv,
            args: vec![
                "run".into(),
                "--no-sync".into(),
                "--directory".into(),
                sidecar_dir.to_string_lossy().into_owned(),
                "python".into(),
                "-c".into(),
                FAKE_SIDECAR.into(),
            ],
            env,
        })
    }

    fn logged_methods(log: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[tokio::test]
    async fn unload_model_asks_the_sidecar_to_release_its_engines() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("calls.log");
        let Some(spec) = fake_sidecar_spec(&log, "ok") else {
            return;
        };
        let adapter = VisionAdapter::with_spec(spec);
        adapter
            .load_model("dataset-vision-pipeline", 2560)
            .await
            .unwrap();
        assert_eq!(adapter.vram_used_mb(), 2560);

        adapter
            .unload_model("dataset-vision-pipeline")
            .await
            .unwrap();

        assert_eq!(logged_methods(&log), vec![UNLOAD_METHOD.to_string()]);
        assert!(adapter.loaded_models().is_empty());
    }

    #[tokio::test]
    async fn a_sidecar_error_on_unload_still_clears_the_bookkeeping() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("calls.log");
        let Some(spec) = fake_sidecar_spec(&log, "fail") else {
            return;
        };
        let adapter = VisionAdapter::with_spec(spec);
        adapter
            .load_model("dataset-vision-pipeline", 2560)
            .await
            .unwrap();

        // Logged as a warning, not returned: an eviction must not fail the
        // job that needs the room, and the ledger must not keep claiming a
        // model the core asked to drop.
        adapter
            .unload_model("dataset-vision-pipeline")
            .await
            .unwrap();

        assert_eq!(logged_methods(&log), vec![UNLOAD_METHOD.to_string()]);
        assert!(adapter.loaded_models().is_empty());
        assert_eq!(adapter.vram_used_mb(), 0);
    }

    /// A stuck sidecar must not stall the job queue: the engine awaits this
    /// release after every dataset prep.
    #[tokio::test]
    async fn a_stuck_sidecar_on_unload_times_out_and_still_clears_the_bookkeeping() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("calls.log");
        let Some(spec) = fake_sidecar_spec(&log, "hang") else {
            return;
        };
        let adapter = VisionAdapter {
            unload_timeout: Duration::from_millis(300),
            ..VisionAdapter::with_spec(spec)
        };
        adapter
            .load_model("dataset-vision-pipeline", 2560)
            .await
            .unwrap();

        let t0 = std::time::Instant::now();
        adapter
            .unload_model("dataset-vision-pipeline")
            .await
            .unwrap();

        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "waited {:?} on a stuck sidecar",
            t0.elapsed()
        );
        assert_eq!(logged_methods(&log), vec![UNLOAD_METHOD.to_string()]);
        assert!(adapter.loaded_models().is_empty());
    }

    #[tokio::test]
    async fn unload_without_a_running_sidecar_never_spawns_one() {
        let adapter = VisionAdapter::with_spec(SidecarSpec {
            program: "definitely-not-a-real-binary".into(),
            args: vec![],
            env: vec![],
        });

        adapter
            .unload_model("dataset-vision-pipeline")
            .await
            .unwrap();

        assert_eq!(adapter.health().await, Health::Unknown);
    }

    #[tokio::test]
    async fn reports_zero_vram_and_healthy_none_before_any_use() {
        let adapter = VisionAdapter::new();
        assert_eq!(adapter.health().await, Health::Unknown);
        assert_eq!(adapter.vram_used_mb(), 0);
    }
}
