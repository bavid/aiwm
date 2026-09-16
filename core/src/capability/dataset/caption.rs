//! Stage 4: auto-captioning. Florence-2 (Microsoft, MIT, 230M/770M params)
//! captions every kept frame — small and fast enough for thousands of
//! frames. When its caption *looks* low-confidence, the frame is
//! re-captioned with Qwen2.5-VL-7B-Instruct (Alibaba, Apache-2.0), a real
//! multi-image chat VLM, alongside a nearby frame from the same source video
//! for temporal context ("what changed between this frame and one a bit
//! later") — Florence-2 only ever sees one image at a time and cannot do
//! this. Both facts (license, param count, multi-image support) were
//! verified against each model's current Hugging Face model card while
//! building this, not assumed from memory.
//!
//! **Escalation heuristic** (documented per docs/TODO.md's explicit ask for
//! "a real, simple heuristic ... your call"): a caption escalates when *any*
//! of —
//! 1. it is suspiciously short (`< MIN_CAPTION_WORDS` words) — Florence-2's
//!    `<DETAILED_CAPTION>` task normally returns a full sentence; a
//!    much-shorter answer usually means it found little to describe;
//!    2. it contains a hedging phrase (`HEDGE_WORDS`) — Florence-2 has no
//!    "I don't know" refusal, but genuinely uncertain output tends to read
//!    as vague/hedged;
//!    3. its position is a multiple of `escalate_every_nth` — a periodic
//!    safety net so a systematically-fooled heuristic (confidently wrong,
//!    not short or hedged) still gets occasional temporal-context review,
//!    independent of the other two signals.
//!
//! Escalation only ever applies to a frame extracted from a video (it needs
//! a temporal neighbor); a standalone image has none and keeps its
//! Florence-2 caption regardless of how it scores.

use std::path::Path;

use serde_json::{json, Value};

use crate::db::Database;
use crate::runtime::VisionAdapter;
use crate::Result;

use super::dataset_err;

/// Model-library roles a Florence-2 / Qwen2.5-VL checkpoint is imported
/// under (Models tab \u{2192} Add models, same generic "point at a folder,
/// assign a role" flow every other runtime already uses — see
/// `capability::tts::resolve_dia_dirs` for the same shape with Dia).
pub const FLORENCE2_ROLE: &str = "vision_florence2";
pub const QWEN_VL_ROLE: &str = "vision_qwen2_5_vl";

/// fp16 weights (~770M params \u{2248} 1.5 GB) plus activation/runtime
/// overhead for the `large` Florence-2 checkpoint used by default.
pub const FLORENCE2_VRAM_FALLBACK_MB: u64 = 2048;
/// Qwen2.5-VL-7B-Instruct loaded 4-bit (`bitsandbytes`) — the only way a 7B
/// VLM comfortably shares a 16 GB card with everything else AIWM already
/// puts on it. fp16 (~14 GB weights alone) is deliberately not the default
/// for that reason; `quantization` stays a request-level knob (see
/// `caption_frame_pair`'s `quantization` param) for a bigger card.
pub const QWEN_VL_VRAM_FALLBACK_MB: u64 = 6144;

pub const DEFAULT_ESCALATE: bool = true;
pub const DEFAULT_ESCALATE_EVERY_NTH: u32 = 20;
/// The user's own example was "frame X vs. frame X+5" (docs/TODO.md) — after
/// this pipeline's own fps-sampling + filtering, "frame" means one *kept*
/// frame, so this offsets by kept-frame position, not raw video frames.
pub const DEFAULT_CONTEXT_OFFSET: usize = 5;

const MIN_CAPTION_WORDS: usize = 4;
const HEDGE_WORDS: [&str; 8] = [
    "unclear",
    "unknown",
    "cannot determine",
    "can't determine",
    "not sure",
    "hard to tell",
    "indistinct",
    "difficult to see",
];

fn vision_err(msg: impl std::fmt::Display) -> crate::CoreError {
    dataset_err(msg)
}

/// The default single-image task Florence-2 is asked to perform. `<CAPTION>`
/// is Florence-2's shortest task; `<DETAILED_CAPTION>` is the documented
/// middle ground between that and `<MORE_DETAILED_CAPTION>` (verbose enough
/// to be useful LoRA training text, fast enough for bulk per-frame use).
pub const FLORENCE2_TASK_PROMPT: &str = "<DETAILED_CAPTION>";

/// The fixed question asked of Qwen2.5-VL when escalating with temporal
/// context — deliberately about *change between the two frames*, the one
/// thing Florence-2's single-image view structurally cannot answer.
pub const TEMPORAL_CONTEXT_QUESTION: &str = "These two images are frames from the same video \
    clip, the second captured a short time after the first. In one or two sentences, describe \
    what is happening in the scene and what action or motion occurs between the two frames.";

/// Whether a Florence-2 caption looks low-confidence enough to re-caption
/// with temporal context. Pure and independently testable — see the module
/// doc for the three signals it checks.
pub fn is_low_confidence_caption(
    caption: &str,
    kept_index: usize,
    escalate_every_nth: u32,
) -> bool {
    let words = caption.split_whitespace().count();
    if words < MIN_CAPTION_WORDS {
        return true;
    }
    let lower = caption.to_ascii_lowercase();
    if HEDGE_WORDS.iter().any(|h| lower.contains(h)) {
        return true;
    }
    escalate_every_nth > 0 && (kept_index + 1) % escalate_every_nth as usize == 0
}

/// The imported Florence-2 / Qwen2.5-VL checkpoint's on-disk snapshot
/// directory, resolved by role — mirrors
/// `capability::tts::resolve_dia_component_dir`'s "any file in the role
/// names the shared directory" shape.
async fn resolve_model_dir(db: &Database, role: &str, label: &str) -> Result<std::path::PathBuf> {
    let files = db.models().for_role(role).await?;
    let first = files.first().ok_or_else(|| {
        vision_err(format!(
            "no {label} imported — import it on the Models tab (Add models \u{2192} point at \
             its downloaded snapshot folder \u{2192} role \u{201c}{role}\u{201d})"
        ))
    })?;
    Path::new(&first.file_path)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| vision_err(format!("{label} file has no parent directory")))
}

pub async fn resolve_florence2_dir(db: &Database) -> Result<std::path::PathBuf> {
    resolve_model_dir(db, FLORENCE2_ROLE, "Florence-2").await
}

pub async fn resolve_qwen_vl_dir(db: &Database) -> Result<std::path::PathBuf> {
    resolve_model_dir(db, QWEN_VL_ROLE, "Qwen2.5-VL").await
}

/// One Florence-2 caption for a single frame.
pub async fn caption_frame(
    vision: &VisionAdapter,
    model_dir: &Path,
    image_path: &Path,
) -> Result<String> {
    let client = vision.client().await?;
    let result = client
        .call(
            "caption_frame",
            json!({
                "image_path": image_path.to_string_lossy(),
                "model_dir": model_dir.to_string_lossy(),
                "task_prompt": FLORENCE2_TASK_PROMPT,
            }),
        )
        .await?;
    extract_caption(&result)
}

/// A temporal-context re-caption from Qwen2.5-VL: `image_path` is the
/// uncertain frame, `context_image_path` a nearby later frame from the same
/// source video.
pub async fn caption_frame_pair(
    vision: &VisionAdapter,
    model_dir: &Path,
    image_path: &Path,
    context_image_path: &Path,
) -> Result<String> {
    let client = vision.client().await?;
    let result = client
        .call(
            "caption_frame_pair",
            json!({
                "image_path": image_path.to_string_lossy(),
                "context_image_path": context_image_path.to_string_lossy(),
                "model_dir": model_dir.to_string_lossy(),
                "question": TEMPORAL_CONTEXT_QUESTION,
            }),
        )
        .await?;
    extract_caption(&result)
}

fn extract_caption(result: &Value) -> Result<String> {
    result
        .get("caption")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| vision_err("the sidecar returned no caption"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;

    #[test]
    fn short_captions_are_flagged_low_confidence() {
        assert!(is_low_confidence_caption("a dog", 0, 0));
        assert!(!is_low_confidence_caption(
            "a golden retriever running across a grassy field",
            0,
            0
        ));
    }

    #[test]
    fn hedging_language_is_flagged_regardless_of_length() {
        assert!(is_low_confidence_caption(
            "it is unclear what is happening in this scene",
            0,
            0
        ));
        assert!(is_low_confidence_caption(
            "the subject is hard to tell apart from the background",
            0,
            0
        ));
    }

    #[test]
    fn every_nth_kept_frame_escalates_even_with_a_confident_caption() {
        let confident = "a red sports car driving down a mountain road at sunset";
        assert!(!is_low_confidence_caption(confident, 0, 20));
        assert!(!is_low_confidence_caption(confident, 18, 20));
        assert!(is_low_confidence_caption(confident, 19, 20)); // 20th frame, 0-indexed
    }

    #[test]
    fn zero_disables_the_periodic_escalation() {
        let confident = "a red sports car driving down a mountain road at sunset";
        for i in 0..100 {
            assert!(!is_low_confidence_caption(confident, i, 0));
        }
    }

    async fn empty_db() -> Database {
        Database::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn resolve_florence2_dir_reports_a_clear_error_when_not_imported() {
        let db = empty_db().await;
        let err = resolve_florence2_dir(&db).await.unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
    }

    #[tokio::test]
    async fn resolve_florence2_dir_finds_the_snapshot_folder() {
        let db = empty_db().await;
        db.models()
            .insert(NewModel {
                name: "florence-2-large".into(),
                format: "safetensors".into(),
                file_path: "E:\\AI\\models\\vision\\florence-2-large\\model.safetensors".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec![FLORENCE2_ROLE.into()],
                ..NewModel::default()
            })
            .await
            .unwrap();

        let dir = resolve_florence2_dir(&db).await.unwrap();
        assert_eq!(
            dir.to_string_lossy().replace('\\', "/"),
            "E:/AI/models/vision/florence-2-large"
        );
    }

    #[tokio::test]
    async fn resolve_qwen_vl_dir_reports_a_clear_error_when_not_imported() {
        let db = empty_db().await;
        let err = resolve_qwen_vl_dir(&db).await.unwrap_err();
        assert!(err.to_string().contains("Qwen2.5-VL"), "{err}");
    }

    #[test]
    fn extract_caption_trims_and_rejects_blank_results() {
        assert_eq!(
            extract_caption(&json!({ "caption": "  a scene  " })).unwrap(),
            "a scene"
        );
        assert!(extract_caption(&json!({ "caption": "   " })).is_err());
        assert!(extract_caption(&json!({})).is_err());
    }
}
