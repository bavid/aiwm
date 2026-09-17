//! The `dataset_prep` request: every knob a job's `params` can carry,
//! resolved exactly once with documented defaults and clamps, so no later
//! stage has to re-validate or second-guess a number.

use std::path::PathBuf;

use serde_json::Value;

use crate::db::DatasetMode;
use crate::Result;

use super::{
    caption, captioner, dataset_err, filter, DEFAULT_BLUR_THRESHOLD, DEFAULT_CONTEXT_OFFSET,
    DEFAULT_ESCALATE, DEFAULT_ESCALATE_EVERY_NTH, DEFAULT_PHASH_MAX_DISTANCE, DEFAULT_SAMPLE_FPS,
    MAX_PHASH_DISTANCE, MAX_SAMPLE_FPS, MIN_SAMPLE_FPS,
};

const MAX_BLUR_THRESHOLD: f64 = filter::MAX_BLUR_THRESHOLD;
const MIN_BLUR_THRESHOLD: f64 = filter::MIN_BLUR_THRESHOLD;
const MAX_ESCALATE_EVERY_NTH: u32 = 500;
const MIN_CONTEXT_OFFSET: usize = 1;
const MAX_CONTEXT_OFFSET: usize = 50;

/// Clip mode: a video shorter than this carries too little to train on and
/// is recorded as [`filter::RejectionReason::Unusable`] instead of kept.
pub const DEFAULT_MIN_CLIP_SECS: f64 = 2.0;
/// Upper bound for the per-clip diversity cap — a request asking for more
/// frames than this is asking for "all of them" anyway.
const MAX_FRAMES_PER_CLIP_CEILING: usize = 10_000;

/// A resolved dataset-prep request, pulled from a job's `params`. Only
/// `root` is required; everything else has a documented default (see the
/// submodule that owns each constant).
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetPrepRequest {
    pub root: PathBuf,
    pub mode: DatasetMode,
    /// A [`captioner::CAPTIONERS`] id, or `None` = extract/filter/curate
    /// only (no model is loaded and no caption is written).
    pub captioner: Option<String>,
    pub sample_fps: f64,
    pub blur_threshold: f64,
    pub phash_max_distance: u32,
    pub max_frames_per_clip: usize,
    pub min_clip_secs: f64,
    pub escalate: bool,
    pub escalate_every_nth: u32,
    pub context_offset: usize,
}

impl DatasetPrepRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let root = params
            .get("root")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| dataset_err("dataset_prep job has no `root` folder"))?;

        let mode = match params.get("mode").and_then(Value::as_str) {
            None => DatasetMode::Frames,
            Some(s) => DatasetMode::parse(s).ok_or_else(|| {
                dataset_err(format!(
                    "unknown dataset mode {s:?} (expected frames or clips)"
                ))
            })?,
        };
        let captioner = match params
            .get("captioner")
            .and_then(Value::as_str)
            .map(str::trim)
        {
            None | Some("") => None,
            Some(id) => {
                captioner::find_captioner(id)
                    .ok_or_else(|| dataset_err(format!("unknown captioner {id:?}")))?;
                Some(id.to_string())
            }
        };
        let max_frames_per_clip = params
            .get("max_frames_per_clip")
            .and_then(Value::as_u64)
            .map_or(filter::DEFAULT_MAX_FRAMES_PER_CLIP, |v| {
                usize::try_from(v)
                    .unwrap_or(MAX_FRAMES_PER_CLIP_CEILING)
                    .min(MAX_FRAMES_PER_CLIP_CEILING)
            });
        let min_clip_secs = params
            .get("min_clip_secs")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_MIN_CLIP_SECS, |v| v.max(0.0));

        let sample_fps = params
            .get("sample_fps")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_SAMPLE_FPS, |v| {
                v.clamp(MIN_SAMPLE_FPS, MAX_SAMPLE_FPS)
            });
        let blur_threshold = params
            .get("blur_threshold")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_BLUR_THRESHOLD, |v| {
                v.clamp(MIN_BLUR_THRESHOLD, MAX_BLUR_THRESHOLD)
            });
        let phash_max_distance = params
            .get("phash_max_distance")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .map_or(DEFAULT_PHASH_MAX_DISTANCE, |v| v.min(MAX_PHASH_DISTANCE));
        let escalate = params
            .get("escalate")
            .and_then(Value::as_bool)
            .unwrap_or(DEFAULT_ESCALATE);
        let escalate_every_nth = params
            .get("escalate_every_nth")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .map_or(DEFAULT_ESCALATE_EVERY_NTH, |v| {
                v.min(MAX_ESCALATE_EVERY_NTH)
            });
        let context_offset = params
            .get("context_offset")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_CONTEXT_OFFSET, |v| {
                (v as usize).clamp(MIN_CONTEXT_OFFSET, MAX_CONTEXT_OFFSET)
            });

        Ok(Self {
            root: PathBuf::from(root),
            mode,
            captioner,
            sample_fps,
            blur_threshold,
            phash_max_distance,
            max_frames_per_clip,
            min_clip_secs,
            escalate,
            escalate_every_nth,
            context_offset,
        })
    }

    /// Write the resolved values back over the job's `params`, same reasoning
    /// as `VideoRequest::apply_to` — the UI shows concrete numbers instead of
    /// "whatever the defaults happened to be".
    pub fn apply_to(&self, params: &mut Value) {
        let Some(obj) = params.as_object_mut() else {
            return;
        };
        obj.insert(
            "root".into(),
            self.root.to_string_lossy().into_owned().into(),
        );
        obj.insert("mode".into(), self.mode.as_str().into());
        obj.insert(
            "captioner".into(),
            self.captioner
                .as_ref()
                .map_or(Value::Null, |id| id.clone().into()),
        );
        obj.insert("sample_fps".into(), self.sample_fps.into());
        obj.insert("blur_threshold".into(), self.blur_threshold.into());
        obj.insert("phash_max_distance".into(), self.phash_max_distance.into());
        obj.insert(
            "max_frames_per_clip".into(),
            self.max_frames_per_clip.into(),
        );
        obj.insert("min_clip_secs".into(), self.min_clip_secs.into());
        obj.insert("escalate".into(), self.escalate.into());
        obj.insert("escalate_every_nth".into(), self.escalate_every_nth.into());
        obj.insert("context_offset".into(), self.context_offset.into());
    }

    /// The VRAM the scheduler should reserve for this job's whole captioning
    /// stage: whatever the chosen captioner needs (0 when none was chosen, or
    /// when it is a CPU-only tagger), plus Qwen2.5-VL's 4-bit footprint only
    /// when escalation is actually requested *and* that captioner supports
    /// it (see `resolve_target`'s `dataset_prep` branch in
    /// `orchestrator::engine`, which reads this).
    pub fn vram_estimate_mb(&self) -> u64 {
        let Some(c) = self
            .captioner
            .as_deref()
            .and_then(captioner::find_captioner)
        else {
            return 0;
        };
        c.vram_mb
            + if self.escalate && c.supports_escalation {
                caption::QWEN_VL_VRAM_FALLBACK_MB
            } else {
                0
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::dataset::{FLORENCE2_VRAM_FALLBACK_MB, QWEN_VL_VRAM_FALLBACK_MB};

    #[test]
    fn from_params_requires_a_root() {
        assert!(DatasetPrepRequest::from_params(&serde_json::json!({})).is_err());
        assert!(DatasetPrepRequest::from_params(&serde_json::json!({ "root": "  " })).is_err());
    }

    #[test]
    fn from_params_fills_defaults() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "E:\\Data\\Ghibli" }))
            .unwrap();
        assert_eq!(r.root, PathBuf::from("E:\\Data\\Ghibli"));
        assert_eq!(r.sample_fps, DEFAULT_SAMPLE_FPS);
        assert_eq!(r.blur_threshold, DEFAULT_BLUR_THRESHOLD);
        assert_eq!(r.phash_max_distance, DEFAULT_PHASH_MAX_DISTANCE);
        assert_eq!(r.escalate, DEFAULT_ESCALATE);
        assert_eq!(r.escalate_every_nth, DEFAULT_ESCALATE_EVERY_NTH);
        assert_eq!(r.context_offset, DEFAULT_CONTEXT_OFFSET);
    }

    #[test]
    fn from_params_clamps_out_of_range_values() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": "x",
            "sample_fps": 999.0,
            "blur_threshold": -5.0,
            "phash_max_distance": 999,
            "escalate_every_nth": 999_999,
            "context_offset": 0,
        }))
        .unwrap();
        assert_eq!(r.sample_fps, MAX_SAMPLE_FPS);
        assert_eq!(r.blur_threshold, MIN_BLUR_THRESHOLD);
        assert_eq!(r.phash_max_distance, MAX_PHASH_DISTANCE);
        assert_eq!(r.escalate_every_nth, MAX_ESCALATE_EVERY_NTH);
        assert_eq!(r.context_offset, MIN_CONTEXT_OFFSET);
    }

    #[test]
    fn apply_to_round_trips_resolved_values() {
        let mut params = serde_json::json!({ "root": "E:\\Data\\Ghibli" });
        let r = DatasetPrepRequest::from_params(&params).unwrap();
        r.apply_to(&mut params);
        assert_eq!(params["sample_fps"], DEFAULT_SAMPLE_FPS);
        assert_eq!(params["escalate"], DEFAULT_ESCALATE);
    }

    #[test]
    fn vram_estimate_adds_qwen_only_when_escalating() {
        let base = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": "x", "captioner": "florence2", "escalate": false
        }))
        .unwrap();
        assert_eq!(base.vram_estimate_mb(), FLORENCE2_VRAM_FALLBACK_MB);

        let with_escalation = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": "x", "captioner": "florence2", "escalate": true
        }))
        .unwrap();
        assert_eq!(
            with_escalation.vram_estimate_mb(),
            FLORENCE2_VRAM_FALLBACK_MB + QWEN_VL_VRAM_FALLBACK_MB
        );
    }
    #[test]
    fn from_params_defaults_to_frames_mode_no_captioner_and_the_diversity_cap() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x" })).unwrap();
        assert_eq!(r.mode, crate::db::DatasetMode::Frames);
        assert_eq!(r.captioner, None);
        assert_eq!(r.max_frames_per_clip, filter::DEFAULT_MAX_FRAMES_PER_CLIP);
        assert_eq!(r.min_clip_secs, DEFAULT_MIN_CLIP_SECS);
        assert_eq!(r.vram_estimate_mb(), 0, "no captioner, no VRAM");
    }

    #[test]
    fn from_params_accepts_a_registry_captioner_and_rejects_unknown_ones() {
        let r = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": "x", "captioner": "florence2", "escalate": true }),
        )
        .unwrap();
        assert_eq!(r.captioner.as_deref(), Some("florence2"));
        assert_eq!(
            r.vram_estimate_mb(),
            FLORENCE2_VRAM_FALLBACK_MB + QWEN_VL_VRAM_FALLBACK_MB
        );

        let tagger = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": "x", "captioner": "wd-eva02-tagger-v3", "escalate": true
        }))
        .unwrap();
        assert_eq!(
            tagger.vram_estimate_mb(),
            0,
            "tagger runs on the CPU and cannot escalate"
        );

        let err = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": "x", "captioner": "nope" }),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown captioner"), "{err}");
    }

    #[test]
    fn from_params_parses_clip_mode_and_clamps_the_cap() {
        let r = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": "x", "mode": "clips", "max_frames_per_clip": 999_999, "min_clip_secs": -3.0
        }))
        .unwrap();
        assert_eq!(r.mode, crate::db::DatasetMode::Clips);
        assert_eq!(r.max_frames_per_clip, MAX_FRAMES_PER_CLIP_CEILING);
        assert_eq!(r.min_clip_secs, 0.0);
        let err =
            DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "mode": "stills" }))
                .unwrap_err();
        assert!(err.to_string().contains("mode"), "{err}");
    }
}
