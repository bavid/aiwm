//! VRAM-aware scheduling (ADR-003).
//!
//! The scheduler decides *how* a job's model gets onto the GPU. It never runs a
//! job itself — the job engine acts on the [`Decision`].

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use serde::Serialize;

use crate::runtime::RuntimeRegistry;

/// Rough driver / desktop VRAM overhead to keep in reserve.
pub const DEFAULT_DRIVER_OVERHEAD_MB: u64 = 1024;
/// Extra safety margin so we never plan to the last megabyte.
pub const DEFAULT_HEADROOM_MB: u64 = 512;

/// What the engine needs to place one job's model.
#[derive(Debug, Clone)]
pub struct PlanRequest {
    pub job_id: String,
    pub runtime_id: String,
    pub model_id: String,
    pub vram_needed_mb: u64,
    /// True when a long-running agent owns this model — it must not be evicted.
    pub is_agent_session: bool,
    /// The GPU driver's actual free VRAM right now (from NVML, `None` when the
    /// caller has no live reading) — covers VRAM other applications are using
    /// that the scheduler's own budget bookkeeping never sees. Caps how much
    /// room `plan` believes is available, so a machine with other GPU load
    /// gets an honest `Blocked` instead of a `LoadThenRun` that then fails
    /// with a real CUDA out-of-memory error.
    pub live_free_vram_mb: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// The model is already loaded where it is needed.
    RunNow,
    /// There is room; load the model then run.
    LoadThenRun,
    /// Unload `victim_model` first, then load and run.
    EvictThenLoad { victim_model: String },
    /// Cannot place the job; a human must decide (pause an agent, pick a smaller
    /// model, or wait).
    Blocked { reason: String },
}

#[async_trait]
pub trait Scheduler: Send + Sync + std::fmt::Debug {
    async fn plan(&self, req: &PlanRequest) -> Decision;

    /// Mark `model_id` as owned by an active session so it is never auto-evicted.
    fn pin(&self, _model_id: &str) {}
    fn unpin(&self, _model_id: &str) {}
    fn is_pinned(&self, _model_id: &str) -> bool {
        false
    }

    /// Total VRAM (MB) the scheduler plans against; `0` = unknown. Used by the
    /// benchmark to judge whether a model fits this machine.
    fn budget_mb(&self) -> u64 {
        0
    }
}

/// One resident model per modality stays loaded; everything else queues, and a
/// pinned (agent-owned) model is never auto-evicted.
#[derive(Debug)]
pub struct HybridScheduler {
    registry: RuntimeRegistry,
    budget_mb: u64,
    driver_overhead_mb: u64,
    headroom_mb: u64,
    pinned: Mutex<BTreeSet<String>>,
}

impl HybridScheduler {
    pub fn new(registry: RuntimeRegistry, budget_mb: u64) -> Self {
        Self {
            registry,
            budget_mb,
            driver_overhead_mb: DEFAULT_DRIVER_OVERHEAD_MB,
            headroom_mb: DEFAULT_HEADROOM_MB,
            pinned: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn with_margins(mut self, driver_overhead_mb: u64, headroom_mb: u64) -> Self {
        self.driver_overhead_mb = driver_overhead_mb;
        self.headroom_mb = headroom_mb;
        self
    }

    /// Total VRAM (MB) the scheduler plans against.
    pub fn budget_mb(&self) -> u64 {
        self.budget_mb
    }

    /// VRAM (MB) available for a new model right now, from the scheduler's own
    /// budget bookkeeping alone — does not know about other applications' GPU
    /// usage. See [`Self::effective_free_mb`] for the live-capped figure `plan`
    /// actually uses.
    pub fn free_mb(&self) -> u64 {
        self.budget_mb
            .saturating_sub(self.driver_overhead_mb)
            .saturating_sub(self.headroom_mb)
            .saturating_sub(self.registry.total_vram_used_mb())
    }

    /// `free_mb`, capped by a live NVML reading when the caller has one. The
    /// budget alone can't see VRAM other processes are holding (a browser, a
    /// game, a leftover ComfyUI process) — without this cap the scheduler would
    /// promise room that isn't really there and the load would fail with a raw
    /// CUDA out-of-memory error instead of an honest `Blocked`.
    fn effective_free_mb(&self, live_free_vram_mb: Option<u64>) -> u64 {
        let budgeted = self.free_mb();
        match live_free_vram_mb {
            Some(live) => budgeted.min(live.saturating_sub(self.headroom_mb)),
            None => budgeted,
        }
    }

    fn pinned(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.pinned.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every loaded model except `keep`, with its VRAM and pin status.
    fn eviction_candidates(&self, keep: &str) -> Vec<(String, u64, bool)> {
        self.registry
            .all()
            .iter()
            .flat_map(|rt| rt.loaded_models())
            .filter(|m| m.model_id != keep)
            .map(|m| {
                let pinned = self.is_pinned(&m.model_id);
                (m.model_id, m.vram_mb, pinned)
            })
            .collect()
    }
}

#[async_trait]
impl Scheduler for HybridScheduler {
    fn pin(&self, model_id: &str) {
        self.pinned().insert(model_id.to_string());
    }

    fn unpin(&self, model_id: &str) {
        self.pinned().remove(model_id);
    }

    fn is_pinned(&self, model_id: &str) -> bool {
        self.pinned().contains(model_id)
    }

    fn budget_mb(&self) -> u64 {
        self.budget_mb
    }

    async fn plan(&self, req: &PlanRequest) -> Decision {
        if let Some(rt) = self.registry.get(&req.runtime_id) {
            if rt
                .loaded_models()
                .iter()
                .any(|m| m.model_id == req.model_id)
            {
                return Decision::RunNow;
            }
        }

        let free = self.effective_free_mb(req.live_free_vram_mb);
        if req.vram_needed_mb <= free {
            return Decision::LoadThenRun;
        }

        let deficit = req.vram_needed_mb - free;
        let candidates = self.eviction_candidates(&req.model_id);

        // Prefer the smallest single non-pinned model that closes the deficit;
        // otherwise the largest non-pinned model. Either way, only commit to it
        // if evicting it would actually leave enough room -- with a live VRAM
        // cap in play, freeing a small resident model may still not be enough
        // (the rest of the shortfall is other applications' GPU usage, which
        // eviction can't touch).
        let mut evictable: Vec<_> = candidates
            .iter()
            .filter(|(_, _, pinned)| !pinned)
            .cloned()
            .collect();
        evictable.sort_by_key(|(_, vram, _)| *vram);

        let victim = evictable
            .iter()
            .find(|(_, vram, _)| *vram >= deficit)
            .or_else(|| evictable.last())
            .filter(|(_, vram, _)| free + vram >= req.vram_needed_mb);

        if let Some((victim, _, _)) = victim {
            return Decision::EvictThenLoad {
                victim_model: victim.clone(),
            };
        }

        Decision::Blocked {
            reason: blocked_reason(req, free, self.free_mb(), &candidates),
        }
    }
}

/// `budget_free_mb` is the scheduler's own budget-only figure (ignoring any
/// live VRAM cap) — it tells us whether the *model itself* is simply too big
/// for this card (no live reading would ever change that), versus a live
/// reading being the actual, situational limiter (other applications).
fn blocked_reason(
    req: &PlanRequest,
    free_mb: u64,
    budget_free_mb: u64,
    candidates: &[(String, u64, bool)],
) -> String {
    let pinned_blockers: Vec<&str> = candidates
        .iter()
        .filter(|(_, _, pinned)| *pinned)
        .map(|(id, _, _)| id.as_str())
        .collect();

    if !pinned_blockers.is_empty() {
        format!(
            "{} MB needed, {} MB free; the VRAM is held by pinned model(s) [{}] from an active agent session — pause it or queue this job",
            req.vram_needed_mb,
            free_mb,
            pinned_blockers.join(", "),
        )
    } else if let Some(live) = req.live_free_vram_mb {
        if req.vram_needed_mb > budget_free_mb {
            // Doesn't fit even in the best case (nothing else loaded, no other
            // app competing) -- blaming other applications would be misleading.
            format!(
                "{} MB needed, but this GPU only has {} MB usable in total — this model doesn't fit \
                 this card no matter what else is running. Try a smaller quant/model.",
                req.vram_needed_mb, budget_free_mb,
            )
        } else {
            format!(
                "{} MB needed, only {} MB actually free on the GPU right now (driver reports {} MB free) \
                 — other running applications are using the rest of the VRAM. Close them, or lower the \
                 resolution/model size, and try again",
                req.vram_needed_mb, free_mb, live,
            )
        }
    } else {
        format!(
            "{} MB needed but only {} MB free and nothing can be evicted",
            req.vram_needed_mb, free_mb
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::runtime::{FakeRuntimeAdapter, RuntimeAdapter, RuntimeRegistry};

    /// Build a registry with one fake runtime holding the given `(model, vram)` pairs.
    async fn registry_with(loaded: &[(&str, u64)]) -> (RuntimeRegistry, Arc<FakeRuntimeAdapter>) {
        let reg = RuntimeRegistry::new();
        let rt = Arc::new(FakeRuntimeAdapter::healthy("llamacpp"));
        reg.register(rt.clone());
        for (model, vram) in loaded {
            rt.load_model(model, *vram).await.unwrap();
        }
        (reg, rt)
    }

    fn req(model: &str, vram: u64, agent: bool) -> PlanRequest {
        req_with_live(model, vram, agent, None)
    }

    fn req_with_live(
        model: &str,
        vram: u64,
        agent: bool,
        live_free_vram_mb: Option<u64>,
    ) -> PlanRequest {
        PlanRequest {
            job_id: "j".into(),
            runtime_id: "llamacpp".into(),
            model_id: model.into(),
            vram_needed_mb: vram,
            is_agent_session: agent,
            live_free_vram_mb,
        }
    }

    /// 16 GB card: budget 16384, overhead 1024, headroom 512 => 14848 MB usable.
    fn scheduler(reg: RuntimeRegistry) -> HybridScheduler {
        HybridScheduler::new(reg, 16_384)
    }

    #[tokio::test]
    async fn scenario_nothing_loaded_fits() {
        let (reg, _) = registry_with(&[]).await;
        let sched = scheduler(reg);
        assert_eq!(
            sched.plan(&req("qwen-14b", 8_000, false)).await,
            Decision::LoadThenRun
        );
    }

    #[tokio::test]
    async fn scenario_model_already_loaded() {
        let (reg, _) = registry_with(&[("qwen-14b", 9_000)]).await;
        let sched = scheduler(reg);
        assert_eq!(
            sched.plan(&req("qwen-14b", 9_000, false)).await,
            Decision::RunNow
        );
    }

    #[tokio::test]
    async fn scenario_evict_unpinned_resident() {
        // 14B (10 GB) loaded, unpinned; new job needs 6 GB. free = 14848-10000 = 4848 < 6000.
        let (reg, _) = registry_with(&[("qwen-14b", 10_000)]).await;
        let sched = scheduler(reg);
        assert_eq!(
            sched.plan(&req("flux", 6_000, false)).await,
            Decision::EvictThenLoad {
                victim_model: "qwen-14b".into()
            }
        );
    }

    #[tokio::test]
    async fn scenario_blocked_when_resident_is_pinned() {
        let (reg, _) = registry_with(&[("qwen-14b", 10_000)]).await;
        let sched = scheduler(reg);
        sched.pin("qwen-14b");

        let decision = sched.plan(&req("flux", 6_000, true)).await;
        match decision {
            Decision::Blocked { reason } => {
                assert!(reason.contains("qwen-14b"));
                assert!(reason.contains("agent session"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scenario_small_model_loads_alongside() {
        // 7B (6 GB) loaded, unpinned; new job needs 6 GB. free = 14848-6000 = 8848 >= 6000.
        let (reg, _) = registry_with(&[("qwen-7b", 6_000)]).await;
        let sched = scheduler(reg);
        assert_eq!(
            sched.plan(&req("upscaler", 6_000, false)).await,
            Decision::LoadThenRun
        );
    }

    #[tokio::test]
    async fn scenario_blocked_when_request_exceeds_whole_budget() {
        let (reg, _) = registry_with(&[]).await;
        let sched = scheduler(reg);
        let decision = sched.plan(&req("giant-70b", 40_000, false)).await;
        assert!(matches!(decision, Decision::Blocked { .. }));
    }

    #[tokio::test]
    async fn pin_and_unpin_round_trip() {
        let (reg, _) = registry_with(&[("m", 1_000)]).await;
        let sched = scheduler(reg);
        assert!(!sched.is_pinned("m"));
        sched.pin("m");
        assert!(sched.is_pinned("m"));
        sched.unpin("m");
        assert!(!sched.is_pinned("m"));
    }

    #[tokio::test]
    async fn scenario_blocked_when_live_vram_is_scarce_despite_budget_headroom() {
        // Nothing loaded (budget says 14848 MB free), but another application is
        // holding most of the real VRAM -- the driver reports only 4000 MB free.
        let (reg, _) = registry_with(&[]).await;
        let sched = scheduler(reg);
        let decision = sched
            .plan(&req_with_live("flux", 6_000, false, Some(4_000)))
            .await;
        match decision {
            Decision::Blocked { reason } => {
                assert!(reason.contains("4000"));
                assert!(reason.contains("other running applications"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scenario_eviction_still_helps_under_a_live_vram_cap() {
        // Live free is only 2000 MB (headroom eats 512 -> effective 1488), but the
        // resident 10 GB model really is on the GPU -- evicting it frees that much
        // for real, so 1488 + 10000 covers the 6000 MB job.
        let (reg, _) = registry_with(&[("qwen-14b", 10_000)]).await;
        let sched = scheduler(reg);
        assert_eq!(
            sched
                .plan(&req_with_live("flux", 6_000, false, Some(2_000)))
                .await,
            Decision::EvictThenLoad {
                victim_model: "qwen-14b".into()
            }
        );
    }

    #[tokio::test]
    async fn blocked_reason_does_not_blame_other_apps_when_the_model_never_fits() {
        // 20 GB job on a 16 GB card: no live reading, no eviction, would ever
        // make this fit -- the message must say so plainly, not point at
        // other running applications (misleading when the model itself is
        // just too big).
        let (reg, _) = registry_with(&[]).await;
        let sched = scheduler(reg);
        let decision = sched
            .plan(&req_with_live("giant-model", 20_000, false, Some(14_000)))
            .await;
        match decision {
            Decision::Blocked { reason } => {
                assert!(reason.contains("doesn't fit this card"));
                assert!(!reason.contains("other running applications"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scenario_eviction_would_not_help_under_a_live_vram_cap() {
        // Live free is only 500 MB (effective ~0 after headroom); the resident
        // model is small (1000 MB) so evicting it still leaves the 6000 MB job
        // short -- must not evict for nothing.
        let (reg, _) = registry_with(&[("small", 1_000)]).await;
        let sched = scheduler(reg);
        let decision = sched
            .plan(&req_with_live("flux", 6_000, false, Some(500)))
            .await;
        assert!(matches!(decision, Decision::Blocked { .. }));
    }
}
