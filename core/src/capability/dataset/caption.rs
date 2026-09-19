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
use crate::model::ModelKind;
use crate::runtime::VisionAdapter;
use crate::Result;

use super::captioner::{complete_dir_for_role, installed_captioner_dir, Captioner, FLORENCE2_ID};
use super::dataset_err;

/// Model-library roles a Florence-2 / Qwen2.5-VL checkpoint is imported
/// under (Models tab \u{2192} Add models, same generic "point at a folder,
/// assign a role" flow every other runtime already uses — see
/// `capability::tts::resolve_dia_dirs` for the same shape with Dia).
/// Defined next to their [`crate::model::ModelKind`]s so a catalog stack
/// install and this resolver can never disagree on the string.
pub use crate::model::{FLORENCE2_ROLE, QWEN_VL_ROLE};

/// fp16 weights (~770M params \u{2248} 1.5 GB) plus activation/runtime
/// overhead for the `large` Florence-2 checkpoint used by default.
pub const FLORENCE2_VRAM_FALLBACK_MB: u64 = 2048;
/// Qwen2.5-VL-7B-Instruct loaded 4-bit (`bitsandbytes`) — the only way a 7B
/// VLM comfortably shares a 16 GB card with everything else AIWM already
/// puts on it. fp16 (~14 GB weights alone) is deliberately not the default
/// for that reason; `quantization` stays a request-level knob (see
/// `caption_frame_pair`'s `quantization` param) for a bigger card.
pub const QWEN_VL_VRAM_FALLBACK_MB: u64 = 6144;

/// Every file `Florence2ForConditionalGeneration` + `AutoProcessor` read
/// from the snapshot directory (`vision.py::_construct_florence2`, both with
/// `trust_remote_code`): weights, configs, the tokenizer, and the three
/// remote-code modules -- exactly the pinned `florence2-large` stack.
pub const FLORENCE2_REQUIRED_FILES: &[&str] = &[
    "config.json",
    "model.safetensors",
    "configuration_florence2.py",
    "modeling_florence2.py",
    "processing_florence2.py",
    "preprocessor_config.json",
    "generation_config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "vocab.json",
];

/// Every file `Qwen2_5_VLForConditionalGeneration` + `AutoProcessor` read
/// (`vision.py::_construct_qwen_vl`): the sharded weights with their index,
/// configs, chat template and tokenizer -- exactly the pinned
/// `qwen2.5-vl-7b` stack.
pub const QWEN_VL_REQUIRED_FILES: &[&str] = &[
    "config.json",
    "model.safetensors.index.json",
    "model-00001-of-00005.safetensors",
    "model-00002-of-00005.safetensors",
    "model-00003-of-00005.safetensors",
    "model-00004-of-00005.safetensors",
    "model-00005-of-00005.safetensors",
    "generation_config.json",
    "preprocessor_config.json",
    "chat_template.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "vocab.json",
    "merges.txt",
];

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

/// The shared "not installed" error both `resolve_qwen_vl_dir` and
/// `resolve_captioner_dir` report — same wording, same Models-tab pointer,
/// just parameterized by what's missing and which role to assign it.
fn not_imported_err(label: &str, role: &str) -> crate::CoreError {
    vision_err(format!(
        "no complete {label} installed — install it on the Models tab (Training & captioning), \
         or import its downloaded snapshot folder with role \u{201c}{role}\u{201d}"
    ))
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

/// The imported Qwen2.5-VL snapshot directory: the one directory among the
/// role's rows that holds every one of [`QWEN_VL_REQUIRED_FILES`] -- the
/// same complete-directory rule captioners use, so a half-finished stack
/// download is reported as missing rather than handed to the sidecar.
///
/// The directory handed to the sidecar is then **not** the rows' parent but
/// the store's own `vision/qwen2.5-vl-7b`, after
/// [`crate::model::verify_captioner_dir`] proved it holds exactly the pinned
/// catalog files (no extra file, no subfolder, every SHA-256 matching).
pub async fn resolve_qwen_vl_dir(db: &Database, store_root: &Path) -> Result<std::path::PathBuf> {
    complete_dir_for_role(db, QWEN_VL_ROLE, QWEN_VL_REQUIRED_FILES)
        .await?
        .ok_or_else(|| not_imported_err("Qwen2.5-VL", QWEN_VL_ROLE))?;
    verified_store_dir(store_root, ModelKind::QwenVlEngine).await
}

/// The pinned snapshot kind a captioner loads from, when it has one — the
/// captioners whose folder must pass the load-time integrity check.
fn pinned_kind_for(c: &Captioner) -> Option<ModelKind> {
    (c.id == FLORENCE2_ID).then_some(ModelKind::Florence2Engine)
}

/// Load-time integrity check (`model::integrity`): the verified store
/// subfolder, or a dataset error naming what is wrong with it.
async fn verified_store_dir(store_root: &Path, kind: ModelKind) -> Result<std::path::PathBuf> {
    crate::model::verify_captioner_dir_async(store_root, kind)
        .await
        .map_err(vision_err)
}

/// Directory holding the captioner's files, or the same "import it on the
/// Models tab" error `resolve_model_dir` gives. "Installed" means every one
/// of the captioner's `required_files` sits in one directory (tagger:
/// `model.onnx` + `selected_tags.csv`), so a half-imported tagger is
/// reported as missing rather than resolved to a directory that will fail
/// at caption time.
///
/// For a captioner with a pinned snapshot (Florence-2, which runs its
/// folder's Python via `trust_remote_code`), the returned directory is the
/// store's own subfolder after the load-time integrity check -- never the
/// rows' parent, which any role assignment could point anywhere.
pub async fn resolve_captioner_dir(
    db: &Database,
    store_root: &Path,
    c: &Captioner,
) -> Result<std::path::PathBuf> {
    // Registry display names follow "Name (style hint)" — the label is just
    // the name part.
    let label = c.name.split_once(" (").map_or(c.name, |(label, _)| label);
    let dir = installed_captioner_dir(db, c)
        .await?
        .ok_or_else(|| not_imported_err(label, c.role))?;
    match pinned_kind_for(c) {
        Some(kind) => verified_store_dir(store_root, kind).await,
        None => Ok(dir),
    }
}

/// Which sidecar JSON-RPC method serves this captioner.
pub fn rpc_method_for(c: &Captioner) -> &'static str {
    if c.id == FLORENCE2_ID {
        "caption_frame"
    } else {
        "tag_frame"
    }
}

/// One auto caption for a single frame from whichever captioner the run
/// picked. Returns `(caption, engine_label)`; the label is what lands in
/// `dataset_frames.caption_engine`.
pub async fn caption_with(
    vision: &VisionAdapter,
    c: &Captioner,
    model_dir: &Path,
    image_path: &Path,
) -> Result<(String, String)> {
    let client = vision.client().await?;
    let mut params = json!({
        "image_path": image_path.to_string_lossy(),
        "model_dir": model_dir.to_string_lossy(),
    });
    if c.id == FLORENCE2_ID {
        params["task_prompt"] = json!(FLORENCE2_TASK_PROMPT);
    }
    let result = client.call(rpc_method_for(c), params).await?;
    let engine = result
        .get("engine")
        .and_then(Value::as_str)
        .unwrap_or(c.id)
        .to_string();
    Ok((extract_caption(&result)?, engine))
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
    async fn resolve_qwen_vl_dir_reports_a_clear_error_when_not_imported() {
        let db = empty_db().await;
        let err = resolve_qwen_vl_dir(&db, Path::new("E:\\AI\\models"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Qwen2.5-VL"), "{err}");
    }

    fn qwen_row(name: &str) -> NewModel {
        NewModel {
            name: name.into(),
            format: "safetensors".into(),
            file_path: format!("E:\\AI\\models\\vision\\qwen2.5-vl-7b\\{name}"),
            size_bytes: 1,
            source: "manual".into(),
            roles: vec![QWEN_VL_ROLE.into()],
            ..NewModel::default()
        }
    }

    /// Complete rows are not enough: the folder the sidecar will load is
    /// the store's own `vision/qwen2.5-vl-7b`, and it must pass the pinned
    /// integrity check (`model::integrity`) before escalation may use it.
    #[tokio::test]
    async fn resolve_qwen_vl_dir_requires_the_verified_store_folder() {
        let db = empty_db().await;
        for name in QWEN_VL_REQUIRED_FILES {
            db.models().insert(qwen_row(name)).await.unwrap();
        }
        let store = tempfile::tempdir().unwrap();

        let err = resolve_qwen_vl_dir(&db, store.path()).await.unwrap_err();
        let e = err.to_string();
        assert!(e.contains("captioner folder check failed"), "{e}");
        assert!(e.contains("not installed"), "{e}");
    }

    /// A stack download that has landed one shard must not be handed to
    /// the sidecar as a loadable checkpoint.
    #[tokio::test]
    async fn resolve_qwen_vl_dir_rejects_a_partial_snapshot() {
        let db = empty_db().await;
        db.models()
            .insert(qwen_row("model-00001-of-00005.safetensors"))
            .await
            .unwrap();
        db.models().insert(qwen_row("config.json")).await.unwrap();

        let err = resolve_qwen_vl_dir(&db, Path::new("E:\\AI\\models"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Qwen2.5-VL"), "{err}");
    }

    fn florence_row(dir: &Path, name: &str) -> NewModel {
        NewModel {
            name: name.into(),
            format: "json".into(),
            file_path: dir.join(name).to_string_lossy().into_owned(),
            size_bytes: 1,
            source: "manual".into(),
            roles: vec![FLORENCE2_ROLE.into()],
            ..NewModel::default()
        }
    }

    /// Any row can carry the Florence-2 role (`PUT /models/{id}/roles`,
    /// `register_directory_model`), so a folder whose rows look complete
    /// is still refused when its content is not the pinned snapshot -- the
    /// check that guards `trust_remote_code` runs at load time.
    #[tokio::test]
    async fn resolve_captioner_dir_refuses_a_florence2_folder_that_fails_the_integrity_check() {
        use super::super::captioner::find_captioner;
        let db = empty_db().await;
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join("vision").join("florence2-large");
        std::fs::create_dir_all(&dir).unwrap();
        for name in FLORENCE2_REQUIRED_FILES {
            std::fs::write(dir.join(name), b"not the pinned bytes").unwrap();
            db.models().insert(florence_row(&dir, name)).await.unwrap();
        }
        let florence = find_captioner(FLORENCE2_ID).unwrap();

        let err = resolve_captioner_dir(&db, store.path(), florence)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("captioner folder check failed"),
            "{err}"
        );

        // A registered folder outside the store is never the one loaded.
        let elsewhere = tempfile::tempdir().unwrap();
        let err = resolve_captioner_dir(&db, elsewhere.path(), florence)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not installed"), "{err}");
    }

    /// Real-bytes positive path: set `AIWM_TEST_FLORENCE2_SNAPSHOT` to a
    /// folder holding the ten pinned Florence-2 files (the Plan 7 Task 1
    /// download). Copies ~1.5 GB, hence ignored by default.
    #[tokio::test]
    #[ignore = "needs the real pinned Florence-2 snapshot (AIWM_TEST_FLORENCE2_SNAPSHOT)"]
    async fn resolve_captioner_dir_accepts_the_real_pinned_florence2_snapshot() {
        use super::super::captioner::find_captioner;
        let src = std::path::PathBuf::from(
            std::env::var("AIWM_TEST_FLORENCE2_SNAPSHOT").expect("set the snapshot path"),
        );
        let db = empty_db().await;
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join("vision").join("florence2-large");
        std::fs::create_dir_all(&dir).unwrap();
        for name in FLORENCE2_REQUIRED_FILES {
            std::fs::copy(src.join(name), dir.join(name)).unwrap();
            db.models().insert(florence_row(&dir, name)).await.unwrap();
        }
        let florence = find_captioner(FLORENCE2_ID).unwrap();

        let got = resolve_captioner_dir(&db, store.path(), florence)
            .await
            .unwrap();
        assert_eq!(got, store.path().join("vision/florence2-large"));

        // A stray `__pycache__` (what running the remote code leaves behind
        // if it were ever written here) is refused by name.
        std::fs::create_dir(dir.join("__pycache__")).unwrap();
        let err = resolve_captioner_dir(&db, store.path(), florence)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("__pycache__"), "{err}");
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

    #[tokio::test]
    async fn resolve_captioner_dir_uses_the_registry_role() {
        use super::super::captioner::{find_captioner, FLORENCE2_ID, WD_TAGGER_ID};
        let db = empty_db().await;
        // Both companion files must be present (Task 6 review decision):
        // the tag list alone is "not installed".
        for (name, format) in [("selected_tags.csv", "csv"), ("model.onnx", "onnx")] {
            db.models()
                .insert(NewModel {
                    name: name.into(),
                    format: format.into(),
                    file_path: format!("E:\\AI\\models\\vision\\wd-tagger\\{name}"),
                    size_bytes: 1,
                    source: "manual".into(),
                    roles: vec![crate::model::WD_TAGGER_ROLE.into()],
                    ..NewModel::default()
                })
                .await
                .unwrap();
        }
        let c = find_captioner(WD_TAGGER_ID).unwrap();
        let dir = resolve_captioner_dir(&db, Path::new("E:\\AI\\models"), c)
            .await
            .unwrap();
        assert!(dir
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("vision/wd-tagger"));

        let florence = find_captioner(FLORENCE2_ID).unwrap();
        let err = resolve_captioner_dir(&db, Path::new("E:\\AI\\models"), florence)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
    }

    #[tokio::test]
    async fn resolve_captioner_dir_rejects_a_half_imported_tagger() {
        use super::super::captioner::{find_captioner, WD_TAGGER_ID};
        let db = empty_db().await;
        db.models()
            .insert(NewModel {
                name: "selected_tags.csv".into(),
                format: "csv".into(),
                file_path: "E:\\AI\\models\\vision\\wd-tagger\\selected_tags.csv".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec![crate::model::WD_TAGGER_ROLE.into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        let c = find_captioner(WD_TAGGER_ID).unwrap();
        let err = resolve_captioner_dir(&db, Path::new("E:\\AI\\models"), c)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("WD EVA02 Tagger v3"), "{err}");
    }

    #[test]
    fn rpc_method_and_engine_label_follow_the_captioner() {
        use super::super::captioner::{find_captioner, FLORENCE2_ID, WD_TAGGER_ID};
        let wd = find_captioner(WD_TAGGER_ID).unwrap();
        assert_eq!(rpc_method_for(wd), "tag_frame");
        let fl = find_captioner(FLORENCE2_ID).unwrap();
        assert_eq!(rpc_method_for(fl), "caption_frame");
    }
}
