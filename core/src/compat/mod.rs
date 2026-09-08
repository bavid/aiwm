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

/// Context length the chat capability targets by default. `llama-server` would
/// otherwise allocate the model's full trained context (often 128K), whose KV
/// cache alone can exceed the card — so we cap here and size the estimate to
/// match. The Settings UI will expose an override (slice 2.7).
pub const DEFAULT_CHAT_CTX: u32 = 8192;

/// Flat VRAM overhead for the CUDA context, cuBLAS workspace and compute graph.
const RUNTIME_OVERHEAD_MB: u64 = 650;

/// KV-cache guess per 1K context tokens when the GGUF lacks architecture dims.
/// Generous on purpose — a dense-attention 13B sits near this; GQA models are
/// well under it.
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
}
