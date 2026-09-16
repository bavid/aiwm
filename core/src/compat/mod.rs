//! Will this model fit in VRAM? A rough-but-honest estimate of what
//! `llama-server` needs to load a GGUF at a given context length, so the engine
//! can refuse a job with a plain-language reason *before* paying for a model
//! load that would only OOM.
//!
//! The three parts:
//! - **weights** — the quantized tensors, i.e. the file on disk.
//! - **KV cache** — `fp16`, `K` + `V`, sized from the GGUF's architecture
//!   metadata (`block_count`, `embedding_length`, `attention.head_count[_kv]`)
//!   and the context length. Falls back to a coarse per-1K-token figure when the
//!   header didn't carry those.
//! - **overhead** — CUDA context, cuBLAS workspaces, the compute graph. A flat
//!   constant; calibration against real measurements is a Phase-6 job.
//!
//! Deliberately conservative: better to over-reserve and queue than to load and
//! crash. Not a benchmark — see `docs/HARDWARE.md` for the real numbers.
//!
//! [`verdict`] wraps [`estimate`] into a plain-language 🟢/🟡/🔴 answer for
//! "would this model run on this machine?" — used by Discovery (6.2) and the
//! Upgrade-Check (6.7); it also weighs free system RAM for the offload case.

use serde::Serialize;

/// Context length the chat capability targets by default. `llama-server` would
/// otherwise allocate the model's full trained context (often 128K), whose KV
/// cache alone can exceed the card — so we cap here and size the estimate to
/// match. The Settings UI will expose an override (slice 2.7).
pub const DEFAULT_CHAT_CTX: u32 = 8192;

/// Below this fraction of the VRAM budget a fit is "green"; above it (but still
/// within budget) it is "yellow" — no head-room for a longer context or a
/// second resident model.
const FIT_TIGHT_PCT: u64 = 85;

/// RAM to keep free for the OS + page cache while layers are offloaded there.
const OFFLOAD_RAM_RESERVE_MB: u64 = 4096;

/// A plain-language answer to "will this model run on this machine?" — for
/// Discovery (6.2) and the Upgrade-Check (6.7). Advisory: it is **not** the
/// scheduler's live "does it fit right now given what's loaded" check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum FitVerdict {
    /// Comfortably within the VRAM budget.
    Green,
    /// Runs, with a caveat (tight, or partly offloaded to system RAM).
    Yellow { reason: String },
    /// Will not run acceptably here.
    Red { reason: String },
    /// No budget set, or not enough information to judge.
    Unknown,
}

/// Judge `dims` at context length `ctx` against the VRAM budget and the amount
/// of free system RAM (for the offload fallback).
pub fn verdict(dims: &ModelDims, ctx: u32, vram_budget_mb: u64, free_ram_mb: u64) -> FitVerdict {
    if dims.size_bytes == 0 {
        return FitVerdict::Unknown;
    }
    verdict_from_total_mb(estimate(dims, ctx).total_mb, vram_budget_mb, free_ram_mb)
}

/// Judge an already-computed total VRAM figure (weights + KV + overhead)
/// against the budget + free RAM. [`verdict`] is this fed from a GGUF/
/// safetensors file's real dims; a caller that only has a rough all-in
/// estimate — the curated chat/coding picks in `core::model::catalog`, sized
/// from public numbers rather than a live file — can call this directly.
pub fn verdict_from_total_mb(total_mb: u64, vram_budget_mb: u64, free_ram_mb: u64) -> FitVerdict {
    if vram_budget_mb == 0 || total_mb == 0 {
        return FitVerdict::Unknown;
    }

    if total_mb <= vram_budget_mb {
        if total_mb.saturating_mul(100) > vram_budget_mb.saturating_mul(FIT_TIGHT_PCT) {
            return FitVerdict::Yellow {
                reason: format!(
                    "needs ~{} of your ~{} VRAM budget — little head-room for a longer \
                     context or a second resident model",
                    gb(total_mb),
                    gb(vram_budget_mb),
                ),
            };
        }
        return FitVerdict::Green;
    }

    // Over the VRAM budget — the runtime can offload layers to system RAM if
    // there is room, at a large speed cost.
    let overflow = total_mb - vram_budget_mb;
    if free_ram_mb >= overflow.saturating_add(OFFLOAD_RAM_RESERVE_MB) {
        FitVerdict::Yellow {
            reason: format!(
                "needs ~{}, over your ~{} VRAM budget — about {} would run in system RAM \
                 (much slower)",
                gb(total_mb),
                gb(vram_budget_mb),
                gb(overflow),
            ),
        }
    } else {
        FitVerdict::Red {
            reason: format!(
                "needs ~{}, over your ~{} VRAM budget, and only ~{} RAM is free to offload \
                 the rest",
                gb(total_mb),
                gb(vram_budget_mb),
                gb(free_ram_mb),
            ),
        }
    }
}

/// Flat VRAM overhead for the CUDA context, cuBLAS workspace and compute graph.
///
/// Calibrated (2026-09-16) against a real `llama-server` load on this
/// project's actual RTX 4080 Super (16 GB), `-ngl 999` (full GPU offload):
/// Mistral-Small-3.2-24B-Instruct, IQ3_M GGUF (10650964832 bytes on disk),
/// 40 layers / 5120 hidden / 32 heads / 8 KV heads, served at the 8192-token
/// chat-default context. The old 650 MB predicted 12407 MB total (weights
/// 10157 + KV 1600 + 650); NVML showed the process's actual VRAM delta
/// (idle-desktop baseline 909 MB \u{2192} peak 12599 MB while resident) was
/// only 11690 MB \u{2014} weights + KV alone (11757 MB) already slightly
/// *exceed* what was actually used, so the true fixed overhead for this run
/// was near zero, not 650 MB. Lowered to 350 MB: still a real, positive
/// safety margin (roughly a CUDA context's worth) rather than 0, but closes
/// most of the 717 MB gap the old constant left on the table. Single
/// real-hardware sample — the other real GGUF chat model on this machine
/// (Qwen2.5-7B-Instruct, F16) doesn't fit this card at all (needs ~15.3 GB of
/// 16 GB), so it couldn't be used as a second data point here. Recalibrate
/// further as more real loads (different quant/arch) become available.
const RUNTIME_OVERHEAD_MB: u64 = 350;

/// KV-cache guess per 1K context tokens when the GGUF lacks architecture dims.
/// Generous on purpose — a dense-attention 13B sits near this; GQA models are
/// well under it.
///
/// **Not calibrated against a real measurement**: both real GGUF chat models
/// available on this machine (Qwen2.5-7B-Instruct, Mistral-Small-3.2-24B)
/// carry full architecture metadata (`n_layers`/`n_embd`/`n_heads`/
/// `n_kv_heads`), so every real load in this session hit the *precise*
/// `kv_bytes_per_token` path (see [`ModelDims::kv_bytes_per_token`]), never
/// this rough fallback — it only fires for a GGUF whose header is missing
/// those fields, which none of the imported models are. Left as-is; would
/// need a real model with a stripped/incomplete header to calibrate honestly.
const KV_ROUGH_MB_PER_1K_CTX: u64 = 160;

const MIB: u64 = 1024 * 1024;

/// The architecture facts the estimate needs. Everything is optional: an old
/// import, a non-GGUF format, or a stripped header may not carry the dims, and
/// the estimate degrades to a coarser guess rather than failing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelDims {
    /// The quantized weights on disk, in bytes.
    pub size_bytes: u64,
    /// Trained context length (`<arch>.context_length`).
    pub ctx_max: Option<u32>,
    /// Total parameters — only used for sanity, not the estimate.
    pub param_count: Option<u64>,
    /// Transformer layers (`<arch>.block_count`).
    pub n_layers: Option<u32>,
    /// Hidden size (`<arch>.embedding_length`).
    pub n_embd: Option<u32>,
    /// Attention heads (`<arch>.attention.head_count`).
    pub n_heads: Option<u32>,
    /// KV heads (`<arch>.attention.head_count_kv`); `n_heads` for plain MHA.
    pub n_kv_heads: Option<u32>,
}

impl ModelDims {
    /// Bytes of `fp16` KV cache per context token, when the header gave us
    /// enough detail. `K` and `V` together:
    /// `2 tensors × 2 bytes × layers × head_dim × kv_heads`,
    /// with `head_dim = embedding_length / head_count`.
    fn kv_bytes_per_token(&self) -> Option<u64> {
        let layers = u64::from(self.n_layers?);
        let embd = u64::from(self.n_embd?);
        let heads = u64::from(self.n_heads?);
        let kv_heads = u64::from(self.n_kv_heads.or(self.n_heads)?);
        if layers == 0 || embd == 0 || heads == 0 || kv_heads == 0 {
            return None;
        }
        let head_dim = embd / heads;
        Some(4 * layers * head_dim * kv_heads)
    }
}

/// A VRAM requirement broken into parts the UI and logs can explain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VramEstimate {
    /// Context length this estimate was computed for.
    pub ctx: u32,
    pub weights_mb: u64,
    pub kv_cache_mb: u64,
    pub overhead_mb: u64,
    pub total_mb: u64,
    /// True when the KV figure is the coarse fallback (dims were missing).
    pub kv_is_rough: bool,
}

impl VramEstimate {
    /// One-line plain-language breakdown, e.g.
    /// `~11.8 GB (weights 8.4 GB + KV cache 2.7 GB @ 8K ctx + 0.6 GB overhead)`.
    pub fn describe(&self) -> String {
        let kv = if self.kv_is_rough {
            "KV cache ~"
        } else {
            "KV cache "
        };
        format!(
            "~{} (weights {} + {}{} @ {} ctx + {} overhead)",
            gb(self.total_mb),
            gb(self.weights_mb),
            kv,
            gb(self.kv_cache_mb),
            ctx_label(self.ctx),
            gb(self.overhead_mb),
        )
    }
}

/// The context length to actually run a chat at: the model's trained maximum,
/// but never more than [`DEFAULT_CHAT_CTX`].
pub fn effective_ctx(ctx_max: Option<u32>) -> u32 {
    match ctx_max {
        Some(c) if c > 0 => c.min(DEFAULT_CHAT_CTX),
        _ => DEFAULT_CHAT_CTX,
    }
}

/// Estimate the VRAM `llama-server` needs to serve `dims` at `ctx` tokens.
pub fn estimate(dims: &ModelDims, ctx: u32) -> VramEstimate {
    let ctx = ctx.max(1);
    let weights_mb = dims.size_bytes / MIB;

    let (kv_cache_mb, kv_is_rough) = match dims.kv_bytes_per_token() {
        Some(per_token) => (per_token.saturating_mul(u64::from(ctx)) / MIB, false),
        None => {
            let per_1k = u64::from(ctx)
                .div_ceil(1024)
                .saturating_mul(KV_ROUGH_MB_PER_1K_CTX);
            (per_1k, true)
        }
    };

    let overhead_mb = RUNTIME_OVERHEAD_MB;
    let total_mb = weights_mb
        .saturating_add(kv_cache_mb)
        .saturating_add(overhead_mb);

    VramEstimate {
        ctx,
        weights_mb,
        kv_cache_mb,
        overhead_mb,
        total_mb,
        kv_is_rough,
    }
}

fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

fn ctx_label(ctx: u32) -> String {
    if ctx >= 1024 && ctx % 1024 == 0 {
        format!("{}K", ctx / 1024)
    } else {
        ctx.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB_I: u64 = 1024 * 1024;

    /// Qwen2.5-7B-Instruct: 28 layers, hidden 3584, 28 heads, 4 KV heads (GQA).
    fn qwen2_5_7b() -> ModelDims {
        ModelDims {
            size_bytes: 4400 * MIB_I,
            ctx_max: Some(32768),
            param_count: Some(7_615_616_512),
            n_layers: Some(28),
            n_embd: Some(3584),
            n_heads: Some(28),
            n_kv_heads: Some(4),
        }
    }

    /// The real machine + real model this session's `RUNTIME_OVERHEAD_MB`
    /// recalibration is based on: Mistral-Small-3.2-24B-Instruct, IQ3_M GGUF,
    /// on the project's actual RTX 4080 Super (see the constant's doc comment
    /// for the full real-measurement writeup).
    fn mistral_small_3_2_24b_iq3_m() -> ModelDims {
        ModelDims {
            size_bytes: 10_650_964_832,
            ctx_max: Some(131_072),
            param_count: Some(23_572_403_200),
            n_layers: Some(40),
            n_embd: Some(5120),
            n_heads: Some(32),
            n_kv_heads: Some(8),
        }
    }

    #[test]
    fn real_mistral_load_pins_the_recalibrated_overhead_and_stays_above_measured_actual() {
        // effective_ctx caps the 131K trained context at the 8192 chat default,
        // exactly what actually served the real chat job this was measured on.
        let ctx = effective_ctx(mistral_small_3_2_24b_iq3_m().ctx_max);
        let est = estimate(&mistral_small_3_2_24b_iq3_m(), ctx);

        assert!(!est.kv_is_rough);
        assert_eq!(est.weights_mb, 10_157);
        assert_eq!(est.kv_cache_mb, 1_600);
        assert_eq!(est.overhead_mb, RUNTIME_OVERHEAD_MB);
        assert_eq!(est.total_mb, 10_157 + 1_600 + RUNTIME_OVERHEAD_MB);

        // NVML measured this real load's actual VRAM delta at 11 690 MB
        // (idle-desktop baseline 909 MB -> peak 12 599 MB while resident).
        // The estimate must stay conservative (>= actual) so the scheduler
        // never under-reserves, while no longer overshooting by the old
        // constant's 717 MB.
        const REAL_MEASURED_ACTUAL_MB: u64 = 11_690;
        assert!(
            est.total_mb >= REAL_MEASURED_ACTUAL_MB,
            "estimate {} must not under-predict the real measured {REAL_MEASURED_ACTUAL_MB} MB",
            est.total_mb
        );
        let overshoot = est.total_mb - REAL_MEASURED_ACTUAL_MB;
        assert!(
            overshoot < 500,
            "expected the recalibrated overhead to overshoot by well under the old 717 MB gap, got {overshoot}"
        );
    }

    #[test]
    fn qwen2_5_7b_kv_cache_is_about_450_mb_at_8k() {
        let est = estimate(&qwen2_5_7b(), 8192);
        assert!(!est.kv_is_rough);
        // 4 * 28 * 128 * 4 = 57 344 B/token * 8192 / MiB = 448 MiB.
        assert_eq!(est.kv_cache_mb, 448);
        assert_eq!(est.weights_mb, 4400);
        assert_eq!(est.overhead_mb, RUNTIME_OVERHEAD_MB);
        assert_eq!(est.total_mb, 4400 + 448 + RUNTIME_OVERHEAD_MB);
        assert_eq!(est.ctx, 8192);
    }

    #[test]
    fn llama3_8b_gqa_kv_is_about_1gb_at_8k() {
        // 32 layers, hidden 4096, 32 heads, 8 KV heads.
        let dims = ModelDims {
            size_bytes: 4900 * MIB_I,
            ctx_max: Some(8192),
            n_layers: Some(32),
            n_embd: Some(4096),
            n_heads: Some(32),
            n_kv_heads: Some(8),
            ..ModelDims::default()
        };
        let est = estimate(&dims, 8192);
        // 4 * 32 * 128 * 8 = 131 072 B/token * 8192 / MiB = 1024 MiB.
        assert_eq!(est.kv_cache_mb, 1024);
        assert!(!est.kv_is_rough);
    }

    #[test]
    fn kv_cache_scales_linearly_with_context() {
        let at_8k = estimate(&qwen2_5_7b(), 8192).kv_cache_mb;
        let at_16k = estimate(&qwen2_5_7b(), 16384).kv_cache_mb;
        assert_eq!(at_16k, at_8k * 2);
    }

    #[test]
    fn missing_kv_heads_falls_back_to_head_count() {
        let mut dims = qwen2_5_7b();
        dims.n_kv_heads = None; // treat as dense MHA: 28 KV heads
        let est = estimate(&dims, 8192);
        // 4 * 28 * 128 * 28 = 401 408 B/token * 8192 / MiB = 3136 MiB.
        assert_eq!(est.kv_cache_mb, 3136);
        assert!(!est.kv_is_rough);
    }

    #[test]
    fn missing_dims_use_the_rough_heuristic() {
        let dims = ModelDims {
            size_bytes: 4000 * MIB_I,
            ctx_max: Some(8192),
            ..ModelDims::default()
        };
        let est = estimate(&dims, 8192);
        assert!(est.kv_is_rough);
        assert_eq!(est.kv_cache_mb, 8 * KV_ROUGH_MB_PER_1K_CTX);
        assert_eq!(
            est.total_mb,
            4000 + 8 * KV_ROUGH_MB_PER_1K_CTX + RUNTIME_OVERHEAD_MB
        );
    }

    #[test]
    fn zero_heads_in_the_header_fall_back_to_rough() {
        let dims = ModelDims {
            size_bytes: 100 * MIB_I,
            n_layers: Some(32),
            n_embd: Some(4096),
            n_heads: Some(0),
            ..ModelDims::default()
        };
        assert!(estimate(&dims, 4096).kv_is_rough);
    }

    #[test]
    fn effective_ctx_caps_at_the_default() {
        assert_eq!(effective_ctx(Some(131072)), DEFAULT_CHAT_CTX);
        assert_eq!(effective_ctx(Some(4096)), 4096);
        assert_eq!(effective_ctx(Some(0)), DEFAULT_CHAT_CTX);
        assert_eq!(effective_ctx(None), DEFAULT_CHAT_CTX);
    }

    #[test]
    fn describe_reads_plainly() {
        let text = estimate(&qwen2_5_7b(), 8192).describe();
        assert!(text.contains("weights"), "{text}");
        assert!(text.contains("KV cache"), "{text}");
        assert!(text.contains("8K ctx"), "{text}");
        assert!(text.contains("overhead"), "{text}");
    }

    #[test]
    fn rough_estimate_is_marked_in_the_description() {
        let dims = ModelDims {
            size_bytes: 4000 * MIB_I,
            ..ModelDims::default()
        };
        assert!(estimate(&dims, 8192).describe().contains("KV cache ~"));
    }

    /// Weights only, no arch dims → the estimate uses the rough KV fallback.
    fn weights(mb: u64) -> ModelDims {
        ModelDims {
            size_bytes: mb * MIB_I,
            ..ModelDims::default()
        }
    }

    #[test]
    fn verdict_is_green_with_comfortable_head_room() {
        // 6 GB weights + rough KV + 0.65 overhead ≈ 7.9 GB, well under 16 GB.
        assert_eq!(
            verdict(&weights(6_000), 8192, 16_000, 32_000),
            FitVerdict::Green
        );
    }

    #[test]
    fn verdict_is_yellow_when_it_fits_but_is_tight() {
        // ~13.7 GB total against a 15 GB budget is > 85 % → tight.
        let v = verdict(&weights(12_000), 8192, 15_000, 32_000);
        match v {
            FitVerdict::Yellow { reason } => assert!(reason.contains("head-room"), "{reason}"),
            other => panic!("expected tight yellow, got {other:?}"),
        }
    }

    #[test]
    fn verdict_is_yellow_when_over_budget_but_offloadable() {
        // ~22 GB total, 16 GB budget → 6 GB overflow; 32 GB RAM covers it.
        let v = verdict(&weights(20_000), 8192, 16_000, 32_000);
        match v {
            FitVerdict::Yellow { reason } => assert!(reason.contains("system RAM"), "{reason}"),
            other => panic!("expected offload yellow, got {other:?}"),
        }
    }

    #[test]
    fn verdict_is_red_when_over_budget_and_ram_cannot_offload() {
        // Same 20 GB model, but only 4 GB RAM free — can't offload 6 GB.
        let v = verdict(&weights(20_000), 8192, 16_000, 4_000);
        assert!(matches!(v, FitVerdict::Red { .. }), "{v:?}");
    }

    #[test]
    fn verdict_is_unknown_without_a_budget_or_a_size() {
        assert_eq!(
            verdict(&weights(8_000), 8192, 0, 32_000),
            FitVerdict::Unknown
        );
        assert_eq!(
            verdict(&ModelDims::default(), 8192, 16_000, 32_000),
            FitVerdict::Unknown
        );
    }

    #[test]
    fn verdict_from_total_mb_matches_verdict_on_the_same_total() {
        // `verdict` on 6 GB weights at 8192 ctx lands at the same total
        // `estimate` would report -- feeding that total straight in must agree.
        let total = estimate(&weights(6_000), 8192).total_mb;
        assert_eq!(
            verdict_from_total_mb(total, 16_000, 32_000),
            FitVerdict::Green
        );
        assert_eq!(
            verdict(&weights(6_000), 8192, 16_000, 32_000),
            FitVerdict::Green
        );
    }

    #[test]
    fn verdict_from_total_mb_is_unknown_without_a_budget_or_a_total() {
        assert_eq!(verdict_from_total_mb(8_000, 0, 32_000), FitVerdict::Unknown);
        assert_eq!(
            verdict_from_total_mb(0, 16_000, 32_000),
            FitVerdict::Unknown
        );
    }

    #[test]
    fn verdict_from_total_mb_is_red_when_it_cannot_offload() {
        let v = verdict_from_total_mb(20_000, 16_000, 1_000);
        assert!(matches!(v, FitVerdict::Red { .. }), "{v:?}");
    }
}
