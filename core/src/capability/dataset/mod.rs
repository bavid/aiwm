//! The dataset-prep capability: drive one `job_type=dataset_prep` body — the
//! "bring your own dataset" pipeline turning a folder tree of raw video/
//! images into a curated, captioned dataset in the standard `NNNN.png` +
//! `NNNN.txt` sidecar-file convention most LoRA trainers (kohya-ss/
//! sd-scripts etc.) expect. See docs/TODO.md's "Lokale KI-Trainings-Engine"
//! entry for the full brief — this module is the dataset-prep half only; the
//! training-orchestrator half (actually running a LoRA job) is explicitly
//! out of scope here.
//!
//! Four stages, each in its own submodule:
//! 1. [`ingest`] — walk the root folder; each immediate subfolder is a tag.
//! 2. [`extract`] — sample video into stills via `ffmpeg`; a plain image
//!    file is already a frame.
//! 3. [`filter`] — drop blurry frames (variance of Laplacian) and collapse
//!    runs of near-duplicate frames (perceptual hashing).
//! 4. [`caption`] — Florence-2 captions every kept frame; a low-confidence
//!    caption on a video frame is escalated to Qwen2.5-VL with a nearby
//!    frame for temporal context.
//!
//! Frames land in the `dataset_frames` table for the curation UI to review;
//! [`export_dataset`] writes the curator's final selection to disk.

mod caption;
mod captioner;
mod compose;
mod extract;
mod filter;
mod ingest;

pub use caption::{
    DEFAULT_CONTEXT_OFFSET, DEFAULT_ESCALATE, DEFAULT_ESCALATE_EVERY_NTH,
    FLORENCE2_VRAM_FALLBACK_MB, QWEN_VL_VRAM_FALLBACK_MB,
};
pub use captioner::{
    captioner_statuses, find_captioner, Captioner, CaptionerStatus, CAPTIONERS, FLORENCE2_ID,
    WD_TAGGER_ID,
};
pub use compose::{compose_caption, token_warning, CaptionOrder, CaptionStyle, ConceptPart};
pub use extract::{DEFAULT_SAMPLE_FPS, MAX_SAMPLE_FPS, MIN_SAMPLE_FPS};
pub use filter::RejectionReason;
pub use filter::{DEFAULT_BLUR_THRESHOLD, DEFAULT_PHASH_MAX_DISTANCE, MAX_PHASH_DISTANCE};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokio::sync::watch;

use crate::db::{Database, DatasetFrame, EventLevel, NewDatasetFrame};
use crate::runtime::VisionAdapter;
use crate::{CoreError, Result};

fn dataset_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "dataset".into(),
        message: msg.to_string(),
    }
}

const MAX_BLUR_THRESHOLD: f64 = filter::MAX_BLUR_THRESHOLD;
const MIN_BLUR_THRESHOLD: f64 = filter::MIN_BLUR_THRESHOLD;
const MAX_ESCALATE_EVERY_NTH: u32 = 500;
const MIN_CONTEXT_OFFSET: usize = 1;
const MAX_CONTEXT_OFFSET: usize = 50;

/// A resolved dataset-prep request, pulled from a job's `params`. Only
/// `root` is required; everything else has a documented default (see the
/// submodule that owns each constant).
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetPrepRequest {
    pub root: PathBuf,
    pub sample_fps: f64,
    pub blur_threshold: f64,
    pub phash_max_distance: u32,
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
            sample_fps,
            blur_threshold,
            phash_max_distance,
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
        obj.insert("sample_fps".into(), self.sample_fps.into());
        obj.insert("blur_threshold".into(), self.blur_threshold.into());
        obj.insert("phash_max_distance".into(), self.phash_max_distance.into());
        obj.insert("escalate".into(), self.escalate.into());
        obj.insert("escalate_every_nth".into(), self.escalate_every_nth.into());
        obj.insert("context_offset".into(), self.context_offset.into());
    }

    /// The VRAM the scheduler should reserve for this job's whole captioning
    /// stage: Florence-2 always, plus Qwen2.5-VL's 4-bit footprint only when
    /// escalation is actually requested (see `resolve_target`'s
    /// `dataset_prep` branch in `orchestrator::engine`, which reads this).
    pub fn vram_estimate_mb(&self) -> u64 {
        caption::FLORENCE2_VRAM_FALLBACK_MB
            + if self.escalate {
                caption::QWEN_VL_VRAM_FALLBACK_MB
            } else {
                0
            }
    }
}

/// One candidate still, before filtering — either an extracted video frame
/// (has a timestamp) or a plain image file (does not).
#[derive(Debug, Clone)]
struct FrameCandidate {
    path: PathBuf,
    source: PathBuf,
    tag: String,
    timestamp_secs: Option<f64>,
}

/// All candidates from one source file (a video's extracted frames, or a
/// single plain image), kept together because filtering compares within a
/// source and temporal-context captioning needs a same-source neighbor.
struct CandidateGroup {
    source: PathBuf,
    frames: Vec<FrameCandidate>,
}

/// A finished dataset-prep pass.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetPrepDone {
    pub frame_count: usize,
    pub tag_counts: BTreeMap<String, usize>,
}

/// How the dataset-prep body came to rest.
#[derive(Debug, Clone)]
pub enum DatasetPrepOutcome {
    Done(DatasetPrepDone),
    Cancelled,
}

fn video_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip")
        .to_string()
}

async fn cancelled(cancel: &watch::Receiver<bool>) -> bool {
    *cancel.borrow()
}

/// Run the whole pipeline for `req`, writing extracted frames under
/// `work_dir/<job_id>/raw/` and persisting kept, captioned frames to the
/// `dataset_frames` table. The vision runtime's model is already loaded by
/// the time this runs (the job engine's generic Target/scheduler flow, same
/// as every other capability) — this function only ever calls
/// `vision.client()` to talk to the already-running sidecar, never
/// `load_model` itself.
pub async fn run(
    db: &Database,
    vision: &VisionAdapter,
    work_dir: &Path,
    job_id: &str,
    req: DatasetPrepRequest,
    cancel: watch::Receiver<bool>,
) -> Result<DatasetPrepOutcome> {
    let florence_dir = caption::resolve_florence2_dir(db).await?;

    let qwen_dir = if req.escalate {
        match caption::resolve_qwen_vl_dir(db).await {
            Ok(dir) => Some(dir),
            Err(e) => {
                db.jobs()
                    .append_event(
                        job_id,
                        EventLevel::Warn,
                        &format!("temporal-context escalation disabled \u{2014} {e}"),
                    )
                    .await?;
                None
            }
        }
    } else {
        None
    };
    let escalate = req.escalate && qwen_dir.is_some();

    let items = ingest::walk_dataset_root(&req.root)?;
    if items.is_empty() {
        return Err(dataset_err(format!(
            "no videos or images found under {} (expected tag subfolders containing .mp4/.png/\
             .jpg/.jpeg/.webp files)",
            req.root.display()
        )));
    }
    let tag_count: std::collections::BTreeSet<&String> = items.iter().map(|i| &i.tag).collect();
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "found {} tag folder(s), {} source file(s)",
                tag_count.len(),
                items.len()
            ),
        )
        .await?;

    let raw_dir = work_dir.join(job_id).join("raw");
    let mut groups: Vec<CandidateGroup> = Vec::new();
    for item in &items {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let frames = match item.kind {
            ingest::IngestKind::Video => {
                let ffmpeg = extract::resolve_ffmpeg().ok_or_else(|| {
                    dataset_err(
                        "ffmpeg was not found on PATH \u{2014} install it to extract frames from video",
                    )
                })?;
                let out_dir = raw_dir.join(&item.tag).join(video_stem(&item.path));
                extract::extract_frames(&ffmpeg, &item.path, &out_dir, req.sample_fps)
                    .await?
                    .into_iter()
                    .map(|f| FrameCandidate {
                        path: f.path,
                        source: item.path.clone(),
                        tag: item.tag.clone(),
                        timestamp_secs: Some(f.timestamp_secs),
                    })
                    .collect()
            }
            ingest::IngestKind::Image => vec![FrameCandidate {
                path: item.path.clone(),
                source: item.path.clone(),
                tag: item.tag.clone(),
                timestamp_secs: None,
            }],
        };
        db.jobs()
            .append_event(
                job_id,
                EventLevel::Info,
                &format!("{} still(s) from {}", frames.len(), item.path.display()),
            )
            .await?;
        groups.push(CandidateGroup {
            source: item.path.clone(),
            frames,
        });
    }

    let Some((kept_groups, total_extracted, total_kept)) = filter_groups(&cancel, groups, &req)?
    else {
        return Ok(DatasetPrepOutcome::Cancelled);
    };
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "kept {total_kept} of {total_extracted} frame(s) after blur/duplicate filtering"
            ),
        )
        .await?;
    if total_kept == 0 {
        return Err(dataset_err(
            "every extracted frame was filtered out as blurry or a duplicate \u{2014} try a \
             lower blur threshold or a higher sample rate",
        ));
    }

    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut frame_count = 0usize;
    for group in &kept_groups {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let mut records: Vec<DatasetFrame> = Vec::with_capacity(group.frames.len());
        for cand in &group.frames {
            let row = db
                .dataset_frames()
                .insert(NewDatasetFrame {
                    job_id: job_id.to_string(),
                    dataset_id: None,
                    tag: cand.tag.clone(),
                    source_path: cand.source.to_string_lossy().into_owned(),
                    frame_path: cand.path.to_string_lossy().into_owned(),
                    timestamp_secs: cand.timestamp_secs,
                    rejection_reason: String::new(),
                    duration_secs: None,
                })
                .await?;
            *tag_counts.entry(cand.tag.clone()).or_insert(0) += 1;
            frame_count += 1;
            records.push(row);
        }

        caption_group(
            db,
            vision,
            &florence_dir,
            qwen_dir.as_deref(),
            escalate,
            &req,
            &records,
            &cancel,
        )
        .await?;
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }

        db.jobs()
            .append_event(
                job_id,
                EventLevel::Info,
                &format!(
                    "captioned {} frame(s) from {}",
                    records.len(),
                    group.source.display()
                ),
            )
            .await?;
    }

    Ok(DatasetPrepOutcome::Done(DatasetPrepDone {
        frame_count,
        tag_counts,
    }))
}

/// Blur, then dedup, within each group independently (see the module docs on
/// [`filter`] for why duplicate detection is scoped per source rather than
/// global). A group that loses every frame to filtering is dropped entirely
/// rather than kept as an empty group. `None` means cancellation was
/// observed partway through.
fn filter_groups(
    cancel: &watch::Receiver<bool>,
    groups: Vec<CandidateGroup>,
    req: &DatasetPrepRequest,
) -> Result<Option<(Vec<CandidateGroup>, usize, usize)>> {
    let mut kept_groups = Vec::new();
    let mut total_extracted = 0usize;
    let mut total_kept = 0usize;
    for group in groups {
        if *cancel.borrow() {
            return Ok(None);
        }
        total_extracted += group.frames.len();

        let mut kept = Vec::new();
        let mut last_hash = None;
        for cand in group.frames {
            if filter::is_blurry(&cand.path, req.blur_threshold)? {
                continue;
            }
            match filter::dedup_step(&cand.path, last_hash.as_ref(), req.phash_max_distance)? {
                Some(hash) => {
                    last_hash = Some(hash);
                    kept.push(cand);
                }
                None => continue,
            }
        }

        total_kept += kept.len();
        if !kept.is_empty() {
            kept_groups.push(CandidateGroup {
                source: group.source,
                frames: kept,
            });
        }
    }
    Ok(Some((kept_groups, total_extracted, total_kept)))
}

/// Caption every frame in one group. Florence-2 first; a low-confidence
/// caption on a video frame (has a timestamp) escalates to Qwen2.5-VL with a
/// same-group neighbor `req.context_offset` positions later, or the group's
/// last frame when the clip is too short for the full offset — unless that
/// neighbor would be the frame itself, in which case escalation is skipped
/// for that one frame (nothing distinct to compare against).
#[allow(clippy::too_many_arguments)]
async fn caption_group(
    db: &Database,
    vision: &VisionAdapter,
    florence_dir: &Path,
    qwen_dir: Option<&Path>,
    escalate: bool,
    req: &DatasetPrepRequest,
    records: &[DatasetFrame],
    cancel: &watch::Receiver<bool>,
) -> Result<()> {
    for (i, record) in records.iter().enumerate() {
        if *cancel.borrow() {
            return Ok(());
        }
        let frame_path = Path::new(&record.frame_path);
        let base_caption = caption::caption_frame(vision, florence_dir, frame_path).await?;
        let mut final_caption = base_caption.clone();
        let mut engine = "florence2";

        let is_video_frame = record.timestamp_secs.is_some();
        if escalate
            && is_video_frame
            && caption::is_low_confidence_caption(&base_caption, i, req.escalate_every_nth)
        {
            let fallback_idx = records.len().saturating_sub(1);
            let neighbor_idx = if i + req.context_offset < records.len() {
                Some(i + req.context_offset)
            } else if fallback_idx != i {
                Some(fallback_idx)
            } else {
                None
            };
            if let (Some(neighbor_idx), Some(qwen_dir)) = (neighbor_idx, qwen_dir) {
                let neighbor_path = Path::new(&records[neighbor_idx].frame_path);
                if let Ok(recap) =
                    caption::caption_frame_pair(vision, qwen_dir, frame_path, neighbor_path).await
                {
                    final_caption = recap;
                    engine = "qwen2.5-vl";
                }
            }
        }

        db.dataset_frames()
            .set_caption(&record.id, &final_caption, engine)
            .await?;
    }
    Ok(())
}

/// What [`export_dataset`] did.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExportSummary {
    pub exported: usize,
    pub dest_dir: PathBuf,
}

/// Write every non-excluded frame from `job_id`'s curation set to
/// `dest_dir` as `NNNN.<ext>` + `NNNN.txt` pairs — the standard sidecar-file
/// convention kohya-ss/sd-scripts and similar LoRA trainers expect. Numbered
/// from `0001` in the same order the curation UI lists them.
pub async fn export_dataset(db: &Database, job_id: &str, dest_dir: &Path) -> Result<ExportSummary> {
    let frames = db.dataset_frames().list_for_job(job_id).await?;
    let kept: Vec<DatasetFrame> = frames.into_iter().filter(|f| !f.excluded).collect();
    if kept.is_empty() {
        return Err(dataset_err(
            "nothing to export \u{2014} every frame is excluded from this dataset",
        ));
    }

    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| dataset_err(format!("create {}: {e}", dest_dir.display())))?;

    for (i, frame) in kept.iter().enumerate() {
        let n = i + 1;
        let ext = Path::new(&frame.frame_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png");
        let image_dest = dest_dir.join(format!("{n:04}.{ext}"));
        tokio::fs::copy(&frame.frame_path, &image_dest)
            .await
            .map_err(|e| {
                dataset_err(format!(
                    "copy {} \u{2192} {}: {e}",
                    frame.frame_path,
                    image_dest.display()
                ))
            })?;

        let caption_dest = dest_dir.join(format!("{n:04}.txt"));
        tokio::fs::write(&caption_dest, frame.caption.as_bytes())
            .await
            .map_err(|e| dataset_err(format!("write {}: {e}", caption_dest.display())))?;
    }

    Ok(ExportSummary {
        exported: kept.len(),
        dest_dir: dest_dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let base =
            DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "escalate": false }))
                .unwrap();
        assert_eq!(base.vram_estimate_mb(), FLORENCE2_VRAM_FALLBACK_MB);

        let with_escalation =
            DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x", "escalate": true }))
                .unwrap();
        assert_eq!(
            with_escalation.vram_estimate_mb(),
            FLORENCE2_VRAM_FALLBACK_MB + QWEN_VL_VRAM_FALLBACK_MB
        );
    }

    #[tokio::test]
    async fn run_reports_a_clear_error_when_florence2_is_not_imported() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let tmp = tempfile::tempdir().unwrap();
        let req = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": tmp.path().to_string_lossy(),
        }))
        .unwrap();
        let (_tx, rx) = watch::channel(false);

        let err = run(&db, &vision, tmp.path(), "job-1", req, rx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
    }

    #[tokio::test]
    async fn export_dataset_reports_a_clear_error_when_nothing_is_kept() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("dataset_prep"))
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();

        let err = export_dataset(&db, &job.id, tmp.path()).await.unwrap_err();
        assert!(err.to_string().contains("nothing to export"), "{err}");
    }

    #[tokio::test]
    async fn export_dataset_writes_numbered_image_and_caption_pairs_skipping_excluded() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("dataset_prep"))
            .await
            .unwrap();
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();

        let mut ids = Vec::new();
        for i in 0..3 {
            let img_path = src.path().join(format!("frame-{i}.png"));
            std::fs::write(&img_path, b"pretend png bytes").unwrap();
            let frame = db
                .dataset_frames()
                .insert(NewDatasetFrame {
                    job_id: job.id.clone(),
                    dataset_id: None,
                    tag: "Ghibli".into(),
                    source_path: "E:\\Data\\Ghibli\\clip.mp4".into(),
                    frame_path: img_path.to_string_lossy().into_owned(),
                    timestamp_secs: Some(f64::from(i)),
                    rejection_reason: String::new(),
                    duration_secs: None,
                })
                .await
                .unwrap();
            db.dataset_frames()
                .set_caption(&frame.id, &format!("caption {i}"), "florence2")
                .await
                .unwrap();
            ids.push(frame.id);
        }
        // Exclude the middle frame.
        db.dataset_frames()
            .set_excluded(&ids[1], true)
            .await
            .unwrap();

        let summary = export_dataset(&db, &job.id, dest.path()).await.unwrap();
        assert_eq!(summary.exported, 2);
        assert!(dest.path().join("0001.png").is_file());
        assert_eq!(
            std::fs::read_to_string(dest.path().join("0001.txt")).unwrap(),
            "caption 0"
        );
        assert!(dest.path().join("0002.png").is_file());
        assert_eq!(
            std::fs::read_to_string(dest.path().join("0002.txt")).unwrap(),
            "caption 2"
        );
    }
}
