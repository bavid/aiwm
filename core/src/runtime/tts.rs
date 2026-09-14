//! TTS runtime adapter — a thin [`RuntimeAdapter`] wrapper around the Python
//! sidecar's `synthesize_speech` (Kokoro ONNX), so `job_type=tts` jobs fit the
//! same Target/scheduler pipeline every other job goes through.
//!
//! Unlike llama.cpp/ComfyUI, there is no VRAM to budget: Kokoro is a tiny
//! (~100-300 MB) CPU model, cached inside the sidecar process itself once
//! loaded there (see `aiwm_sidecar.main._load_kokoro`). This adapter always
//! reports zero VRAM used and never competes for the scheduler's budget — the
//! same idea [`super::ColibriAdapter`] uses for its own RAM-bound models, but
//! simpler: one persistent sidecar process, not a per-model server to swap.

use async_trait::async_trait;
use tokio::sync::OnceCell;

use super::{Health, LoadedModel, RuntimeAdapter, RuntimeKind, SpawnSpec};
use crate::sidecar::{SidecarClient, SidecarSpec};
use crate::Result;

/// Adapter id — also the `runtimes` table key and the registry key.
pub const RUNTIME_ID: &str = "tts";

/// Construction never fails, same as `LlamaCppAdapter`/`ComfyUiAdapter`
/// discovering a binary that may not be installed yet — resolving `uv` and
/// spawning the sidecar is deferred to first real use ([`Self::client`]), so
/// a workstation without `uv` on `PATH` still boots fine; only an actual
/// `tts` job fails, with a clear reason, not the whole app.
#[derive(Debug, Default)]
pub struct TtsAdapter {
    /// `None` (the real constructor) resolves `uv` fresh on first use; tests
    /// pin an explicit spec instead so they never need `uv` on `PATH`.
    spec_override: Option<SidecarSpec>,
    sidecar: OnceCell<SidecarClient>,
}

impl TtsAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_spec(spec: SidecarSpec) -> Self {
        Self {
            spec_override: Some(spec),
            sidecar: OnceCell::new(),
        }
    }

    /// Ensure the sidecar process is up, spawning it on first use. Every
    /// caller shares the one instance; the underlying `uv run` process is
    /// started at most once per adapter.
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
}

#[async_trait]
impl RuntimeAdapter for TtsAdapter {
    fn id(&self) -> &str {
        RUNTIME_ID
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Tts
    }

    /// No process-supervisor spawn: the sidecar manages its own lifecycle via
    /// [`SidecarClient`] (a Job-Object-owned child, killed when the client
    /// drops), not the generic `RuntimeSupervisor` llama.cpp/ComfyUI use.
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

    /// "Loading a model" here just means the sidecar process itself is up —
    /// which specific Kokoro model/voices pair to use is resolved and cached
    /// by `capability::tts::run` on every call, not tracked as a scheduler
    /// slot the way an LLM's resident weights are.
    async fn load_model(&self, _model_id: &str, _vram_mb: u64) -> Result<()> {
        self.client().await.map(|_| ())
    }

    /// Nothing to unload -- the sidecar's own Kokoro cache is an internal
    /// implementation detail, not a scheduler-visible resource.
    async fn unload_model(&self, _model_id: &str) -> Result<()> {
        Ok(())
    }

    /// Always empty -- see the module docs on why this adapter never
    /// competes for VRAM (and so is never a candidate for eviction).
    fn loaded_models(&self) -> Vec<LoadedModel> {
        Vec::new()
    }

    fn detail(&self) -> Option<String> {
        Some(if self.sidecar.initialized() {
            "narrator ready".to_string()
        } else {
            "not started yet".to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_zero_vram_and_no_loaded_models_before_and_after_a_load_attempt() {
        // A bogus program -- this test exercises the *bookkeeping* (never
        // touches VRAM, empty loaded_models), not a real sidecar spawn.
        let adapter = TtsAdapter::with_spec(SidecarSpec {
            program: "definitely-not-a-real-binary".into(),
            args: vec![],
            env: vec![],
        });
        assert_eq!(adapter.id(), "tts");
        assert_eq!(adapter.kind(), RuntimeKind::Tts);
        assert!(adapter.spawn_spec().is_none());
        assert_eq!(adapter.health().await, Health::Unknown);
        assert_eq!(adapter.vram_used_mb(), 0);
        assert!(adapter.loaded_models().is_empty());
        assert_eq!(adapter.detail().as_deref(), Some("not started yet"));

        // The spawn fails (no such binary) -- still zero VRAM, still no
        // loaded models, and unload remains a harmless no-op regardless.
        assert!(adapter.load_model("kokoro", 999_999).await.is_err());
        assert_eq!(adapter.vram_used_mb(), 0);
        assert!(adapter.loaded_models().is_empty());
        assert!(adapter.unload_model("kokoro").await.is_ok());
    }

    #[test]
    fn construction_never_fails_even_without_uv_on_path() {
        // The real constructor -- unlike `SidecarClient::for_dev`, which can
        // fail resolving `uv` immediately, this always succeeds; that
        // resolution is deferred to `client()`.
        let adapter = TtsAdapter::new();
        assert_eq!(adapter.detail().as_deref(), Some("not started yet"));
    }
}
