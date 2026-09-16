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
//! 3. [`filter`] — judge every frame (dead frame, transition, blur,
//!    near-duplicate, per-clip diversity cap) and record *why* each one was
//!    rejected instead of dropping it.
//! 4. [`caption`] — optional: whichever [`captioner`] the request names
//!    captions every kept frame, and (Florence-2 only) a low-confidence
//!    caption on a video frame is escalated to Qwen2.5-VL with a nearby
//!    frame for temporal context. No captioner means no model is loaded at
//!    all — extraction, filtering and curation never need one.
//!
//! A run creates one `datasets` row and files every candidate under it in
//! `dataset_frames` for the curation UI to review. In clips mode there is no
//! frame extraction: each source video becomes one row with its duration and
//! a preview still. [`export_dataset`] writes the curator's final selection
//! to disk, composing each caption from the dataset trigger, the frame's
//! concepts and its own caption (see [`compose`]).

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
    captioner_statuses, find_captioner, installed_captioner_dir, Captioner, CaptionerStatus,
    CAPTIONERS, FLORENCE2_ID, WD_TAGGER_ID,
};
pub use compose::{compose_caption, token_warning, CaptionOrder, CaptionStyle, ConceptPart};
pub use extract::{DEFAULT_SAMPLE_FPS, MAX_SAMPLE_FPS, MIN_SAMPLE_FPS};
pub use filter::RejectionReason;
pub use filter::{DEFAULT_BLUR_THRESHOLD, DEFAULT_PHASH_MAX_DISTANCE, MAX_PHASH_DISTANCE};

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokio::sync::watch;

use crate::db::{Database, DatasetFrame, DatasetMode, EventLevel, NewDatasetFrame};
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
    // Resolve the captioner (and its escalation partner) *before* touching
    // the disk, so a missing model fails fast — but only when one was asked
    // for. Extraction, filtering and curation never need a model.
    let captioner = req.captioner.as_deref().and_then(captioner::find_captioner);
    let captioner_dir = match captioner {
        Some(c) => Some(caption::resolve_captioner_dir(db, c).await?),
        None => None,
    };

    let escalate = req.escalate && captioner.is_some_and(|c| c.supports_escalation);
    let qwen_dir = if escalate {
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
    let escalate = escalate && qwen_dir.is_some();

    let items = ingest::walk_dataset_root(&req.root)?;
    if items.is_empty() {
        return Err(dataset_err(format!(
            "no videos or images found under {} (expected tag subfolders containing .mp4/.png/\
             .jpg/.jpeg/.webp files)",
            req.root.display()
        )));
    }
    let dataset = db
        .datasets()
        .create(crate::db::NewDataset {
            name: req
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "dataset".into()),
            mode: req.mode,
            source_root: req.root.to_string_lossy().into_owned(),
            prep_job_id: Some(job_id.to_string()),
        })
        .await?;

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

    if req.mode == DatasetMode::Clips {
        return run_clip_mode(db, work_dir, job_id, &dataset.id, &items, &req, cancel).await;
    }

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

    let Some((judged_groups, total_extracted, total_kept)) = filter_groups(&cancel, groups, &req)?
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
             lower blur threshold, a higher sample rate or a larger per-clip cap",
        ));
    }

    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut frame_count = 0usize;
    for group in &judged_groups {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        // Every candidate is stored, kept or not: the curation grid filters
        // on `rejection_reason` and can restore an individual frame, which
        // is only possible if the rejected rows exist at all.
        let mut kept_records: Vec<DatasetFrame> = Vec::new();
        for (cand, reason) in &group.frames {
            let row = db
                .dataset_frames()
                .insert(NewDatasetFrame {
                    job_id: job_id.to_string(),
                    dataset_id: Some(dataset.id.clone()),
                    tag: cand.tag.clone(),
                    source_path: cand.source.to_string_lossy().into_owned(),
                    frame_path: cand.path.to_string_lossy().into_owned(),
                    timestamp_secs: cand.timestamp_secs,
                    rejection_reason: reason
                        .map(filter::RejectionReason::as_str)
                        .unwrap_or("")
                        .to_string(),
                    duration_secs: None,
                })
                .await?;
            if reason.is_none() {
                *tag_counts.entry(cand.tag.clone()).or_insert(0) += 1;
                frame_count += 1;
                kept_records.push(row);
            }
        }

        if let (Some(c), Some(dir)) = (captioner, captioner_dir.as_deref()) {
            caption_group(
                db,
                vision,
                c,
                dir,
                qwen_dir.as_deref(),
                escalate,
                &req,
                &kept_records,
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
                        kept_records.len(),
                        group.source.display()
                    ),
                )
                .await?;
        }
    }

    Ok(DatasetPrepOutcome::Done(DatasetPrepDone {
        frame_count,
        tag_counts,
    }))
}

/// Clip mode's one judgement call: a video whose duration ffprobe could not
/// read (undecodable) or that is shorter than the run's minimum is recorded
/// as [`filter::RejectionReason::Unusable`] instead of kept. Pure, so it is
/// testable without ffmpeg installed.
fn clip_is_usable(duration: Option<f64>, min_secs: f64) -> bool {
    duration.is_some_and(|d| d >= min_secs)
}

/// Clip mode (spec 3): no frame extraction at all — each source *video*
/// becomes one dataset row carrying its duration plus a preview still, ready
/// for the curator to set an in/out range that export trims to. Plain images
/// are skipped: there is no clip to trim.
async fn run_clip_mode(
    db: &Database,
    work_dir: &Path,
    job_id: &str,
    dataset_id: &str,
    items: &[ingest::IngestItem],
    req: &DatasetPrepRequest,
    cancel: watch::Receiver<bool>,
) -> Result<DatasetPrepOutcome> {
    let ffmpeg = extract::resolve_ffmpeg().ok_or_else(|| {
        dataset_err("ffmpeg was not found on PATH \u{2014} install it to work with clips")
    })?;
    let ffprobe = extract::resolve_ffprobe()
        .ok_or_else(|| dataset_err("ffprobe was not found next to ffmpeg"))?;
    let preview_dir = work_dir.join(job_id).join("previews");

    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut kept = 0usize;
    for item in items.iter().filter(|i| i.kind == ingest::IngestKind::Video) {
        if cancelled(&cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let duration = extract::probe_duration_secs(&ffprobe, &item.path).await?;
        let usable = clip_is_usable(duration, req.min_clip_secs);
        let preview = preview_dir
            .join(&item.tag)
            .join(format!("{}.png", video_stem(&item.path)));
        if usable {
            // One second in, or the very start for a clip barely that long —
            // the first frame of a cut is often a fade.
            let at = duration.unwrap_or(0.0).min(1.0);
            extract::extract_preview_still(&ffmpeg, &item.path, &preview, at).await?;
        }
        db.dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job_id.to_string(),
                dataset_id: Some(dataset_id.to_string()),
                tag: item.tag.clone(),
                source_path: item.path.to_string_lossy().into_owned(),
                frame_path: if usable {
                    preview.to_string_lossy().into_owned()
                } else {
                    String::new()
                },
                timestamp_secs: None,
                rejection_reason: if usable {
                    String::new()
                } else {
                    filter::RejectionReason::Unusable.as_str().into()
                },
                duration_secs: duration,
            })
            .await?;
        if usable {
            *tag_counts.entry(item.tag.clone()).or_insert(0) += 1;
            kept += 1;
        }
    }

    if kept == 0 {
        return Err(dataset_err(
            "no usable clips \u{2014} every video was undecodable or shorter than the minimum \
             length",
        ));
    }
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!("{kept} usable clip(s) across {} tag(s)", tag_counts.len()),
        )
        .await?;
    Ok(DatasetPrepOutcome::Done(DatasetPrepDone {
        frame_count: kept,
        tag_counts,
    }))
}

/// A group after filtering: every candidate, each with its verdict
/// (`None` = kept).
struct JudgedGroup {
    source: PathBuf,
    frames: Vec<(FrameCandidate, Option<filter::RejectionReason>)>,
}

/// Judge every candidate within each group independently (see the module
/// docs on [`filter`] for why duplicate detection is scoped per source
/// rather than global), in filter level C's spec order — dead frame,
/// transition, blur/duplicate, diversity cap — so a frame that trips more
/// than one check keeps its *first*, most specific reason. Nothing is
/// dropped here: rejected candidates travel on with their reason so the
/// curation grid can show and restore them. `None` means cancellation was
/// observed partway through.
fn filter_groups(
    cancel: &watch::Receiver<bool>,
    groups: Vec<CandidateGroup>,
    req: &DatasetPrepRequest,
) -> Result<Option<(Vec<JudgedGroup>, usize, usize)>> {
    use filter::RejectionReason as R;

    let mut judged_groups = Vec::new();
    let mut total_extracted = 0usize;
    let mut total_kept = 0usize;
    for group in groups {
        if *cancel.borrow() {
            return Ok(None);
        }
        total_extracted += group.frames.len();
        let n = group.frames.len();

        // Pass 1: per-frame facts, decided without looking at neighbours.
        let mut dead = Vec::with_capacity(n);
        let mut blurry = Vec::with_capacity(n);
        let mut hashes = Vec::with_capacity(n);
        for cand in &group.frames {
            dead.push(filter::is_dead_frame(&cand.path)?);
            blurry.push(filter::is_blurry(&cand.path, req.blur_threshold)?);
            hashes.push(filter::phash_of(&cand.path)?);
        }

        // Pass 2: verdicts in spec order; a frame keeps its *first* reason.
        let mut verdict: Vec<Option<R>> = dead.iter().map(|d| d.then_some(R::Black)).collect();
        for (i, v) in verdict.iter_mut().enumerate() {
            if v.is_none()
                && filter::is_transition(
                    blurry[i],
                    i.checked_sub(1).map(|prev| &hashes[prev]),
                    &hashes[i],
                    hashes.get(i + 1),
                    filter::DEFAULT_TRANSITION_MIN_DISTANCE,
                )
            {
                *v = Some(R::Transition);
            }
        }
        let mut last_kept: Option<&image_hasher::ImageHash> = None;
        for (i, v) in verdict.iter_mut().enumerate() {
            if v.is_some() {
                continue;
            }
            if blurry[i] {
                *v = Some(R::Blur);
                continue;
            }
            if let Some(prev) = last_kept {
                if prev.dist(&hashes[i]) <= req.phash_max_distance {
                    *v = Some(R::Duplicate);
                    continue;
                }
            }
            last_kept = Some(&hashes[i]);
        }

        // Pass 3: diversity cap over whatever survived the checks above.
        let survivors: Vec<usize> = verdict
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_none())
            .map(|(i, _)| i)
            .collect();
        let survivor_hashes: Vec<image_hasher::ImageHash> =
            survivors.iter().map(|i| hashes[*i].clone()).collect();
        let chosen = filter::select_diverse(&survivor_hashes, req.max_frames_per_clip);
        for (k, i) in survivors.iter().enumerate() {
            if !chosen.contains(&k) {
                verdict[*i] = Some(R::Cap);
            }
        }

        total_kept += verdict.iter().filter(|v| v.is_none()).count();
        judged_groups.push(JudgedGroup {
            source: group.source,
            frames: group.frames.into_iter().zip(verdict).collect(),
        });
    }
    Ok(Some((judged_groups, total_extracted, total_kept)))
}

/// Caption every kept frame in one group with the run's chosen captioner;
/// a low-confidence caption on a video frame (has a timestamp) escalates to
/// Qwen2.5-VL with a same-group neighbor `req.context_offset` positions
/// later, or the group's last frame when the clip is too short for the full
/// offset — unless that neighbor would be the frame itself, in which case
/// escalation is skipped for that one frame (nothing distinct to compare
/// against). `escalate` already implies `c.supports_escalation`.
#[allow(clippy::too_many_arguments)]
async fn caption_group(
    db: &Database,
    vision: &VisionAdapter,
    c: &Captioner,
    model_dir: &Path,
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
        let (base_caption, mut engine) =
            caption::caption_with(vision, c, model_dir, frame_path).await?;
        let mut final_caption = base_caption.clone();

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
                    engine = "qwen2.5-vl".to_string();
                }
            }
        }

        db.dataset_frames()
            .set_caption(&record.id, &final_caption, &engine)
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

/// What to export, and how to word it.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub dataset_id: String,
    pub dest_dir: PathBuf,
    /// Tags-first (Anime/SDXL) or prose-first (FLUX.2) — see [`compose`].
    pub caption_order: compose::CaptionOrder,
}

/// Write every kept, non-excluded item of one dataset to `dest_dir` as
/// `NNNN.<ext>` + `NNNN.txt` pairs — the standard sidecar-file convention
/// kohya-ss/sd-scripts and similar LoRA trainers expect, numbered from
/// `0001` in the same order the curation UI lists them.
///
/// The caption is *composed* at export time (spec 3A/3D) from the dataset's
/// trigger word, the frame's assigned concepts and its own auto/hand
/// caption — never read back out of `dataset_frames.caption` verbatim. In
/// clips mode the exported media is the source clip itself, stream-copy
/// trimmed when the curator set an in/out range.
pub async fn export_dataset(db: &Database, req: &ExportRequest) -> Result<ExportSummary> {
    let dataset = db
        .datasets()
        .get(&req.dataset_id)
        .await?
        .ok_or_else(|| dataset_err(format!("no such dataset {}", req.dataset_id)))?;
    let frames = db.dataset_frames().list_for_dataset(&dataset.id).await?;
    let kept: Vec<DatasetFrame> = frames
        .into_iter()
        .filter(|f| !f.excluded && f.rejection_reason.is_empty())
        .collect();
    if kept.is_empty() {
        return Err(dataset_err(
            "nothing to export \u{2014} every item is excluded or rejected",
        ));
    }

    let concepts = db.concepts().list_for_dataset(&dataset.id).await?;
    let concept_map = db.concepts().map_for_dataset(&dataset.id).await?;
    let by_id: HashMap<&str, &crate::db::DatasetConcept> =
        concepts.iter().map(|c| (c.id.as_str(), c)).collect();

    tokio::fs::create_dir_all(&req.dest_dir)
        .await
        .map_err(|e| dataset_err(format!("create {}: {e}", req.dest_dir.display())))?;

    let ffmpeg = if dataset.mode == DatasetMode::Clips {
        extract::resolve_ffmpeg()
    } else {
        None
    };

    for (i, frame) in kept.iter().enumerate() {
        let n = i + 1;
        let parts: Vec<compose::ConceptPart<'_>> = concept_map
            .get(&frame.id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| by_id.get(id.as_str()))
                    .map(|c| compose::ConceptPart {
                        token: &c.token,
                        description: &c.description,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let style = captioner::find_captioner(&frame.caption_engine)
            .map_or(compose::CaptionStyle::Prose, |c| c.style);
        let caption = compose::compose_caption(
            req.caption_order,
            &dataset.trigger_word,
            &parts,
            &frame.caption,
            style,
        );

        let src = if dataset.mode == DatasetMode::Clips {
            &frame.source_path
        } else {
            &frame.frame_path
        };
        let ext = Path::new(src)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or(if dataset.mode == DatasetMode::Clips {
                "mp4"
            } else {
                "png"
            });
        let media_dest = req.dest_dir.join(format!("{n:04}.{ext}"));
        match (
            dataset.mode,
            frame.clip_start_secs,
            frame.clip_end_secs,
            ffmpeg.as_deref(),
        ) {
            (DatasetMode::Clips, start, end, Some(ffmpeg)) if start.is_some() || end.is_some() => {
                extract::trim_clip(ffmpeg, Path::new(src), &media_dest, start, end).await?;
            }
            // Silently exporting the *whole* clip when a range was set would
            // hand the trainer data the curator explicitly cut away.
            (DatasetMode::Clips, start, end, None) if start.is_some() || end.is_some() => {
                return Err(dataset_err(
                    "ffmpeg was not found on PATH \u{2014} needed to trim clips with a start/end \
                     range",
                ));
            }
            _ => {
                tokio::fs::copy(src, &media_dest).await.map_err(|e| {
                    dataset_err(format!(
                        "copy {} \u{2192} {}: {e}",
                        src,
                        media_dest.display()
                    ))
                })?;
            }
        }

        let caption_dest = req.dest_dir.join(format!("{n:04}.txt"));
        tokio::fs::write(&caption_dest, caption.as_bytes())
            .await
            .map_err(|e| dataset_err(format!("write {}: {e}", caption_dest.display())))?;
    }

    db.datasets()
        .set_export_dir(&dataset.id, &req.dest_dir.to_string_lossy())
        .await?;
    Ok(ExportSummary {
        exported: kept.len(),
        dest_dir: req.dest_dir.clone(),
    })
}

/// Job-keyed shim for the existing `POST /jobs/{id}/dataset-export` route:
/// resolve the dataset that job produced and export it prose-first.
pub async fn export_dataset_for_job(
    db: &Database,
    job_id: &str,
    dest_dir: &Path,
) -> Result<ExportSummary> {
    let dataset = db
        .datasets()
        .list()
        .await?
        .into_iter()
        .find(|d| d.prep_job_id.as_deref() == Some(job_id))
        .ok_or_else(|| dataset_err("this job produced no dataset"))?;
    export_dataset(
        db,
        &ExportRequest {
            dataset_id: dataset.id,
            dest_dir: dest_dir.to_path_buf(),
            caption_order: compose::CaptionOrder::ProseFirst,
        },
    )
    .await
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

    #[tokio::test]
    async fn run_reports_a_clear_error_when_florence2_is_not_imported() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let tmp = tempfile::tempdir().unwrap();
        let req = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": tmp.path().to_string_lossy(),
            "captioner": "florence2",
        }))
        .unwrap();
        let (_tx, rx) = watch::channel(false);

        let err = run(&db, &vision, tmp.path(), "job-1", req, rx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
    }

    async fn new_job(db: &Database) -> String {
        db.jobs()
            .insert(crate::db::NewJob::new("dataset_prep"))
            .await
            .unwrap()
            .id
    }

    /// A `dataset_prep` job plus the frames-mode dataset it produced.
    async fn frames_dataset(db: &Database) -> (String, crate::db::Dataset) {
        let job_id = new_job(db).await;
        let dataset = db
            .datasets()
            .create(crate::db::NewDataset {
                name: "Ghibli".into(),
                mode: DatasetMode::Frames,
                source_root: "E:\\Data\\Ghibli".into(),
                prep_job_id: Some(job_id.clone()),
            })
            .await
            .unwrap();
        (job_id, dataset)
    }

    /// One frame row with a real file on disk behind it.
    async fn insert_frame(
        db: &Database,
        job_id: &str,
        dataset_id: &str,
        src_dir: &Path,
        i: u32,
        rejection_reason: &str,
    ) -> String {
        let img_path = src_dir.join(format!("frame-{i}.png"));
        std::fs::write(&img_path, b"pretend png bytes").unwrap();
        db.dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job_id.to_string(),
                dataset_id: Some(dataset_id.to_string()),
                tag: "Ghibli".into(),
                source_path: "E:\\Data\\Ghibli\\clip.mp4".into(),
                frame_path: img_path.to_string_lossy().into_owned(),
                timestamp_secs: Some(f64::from(i)),
                rejection_reason: rejection_reason.into(),
                duration_secs: None,
            })
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn export_dataset_reports_a_clear_error_when_nothing_is_kept() {
        let db = Database::connect_in_memory().await.unwrap();
        let (_job_id, dataset) = frames_dataset(&db).await;
        let tmp = tempfile::tempdir().unwrap();

        let err = export_dataset(
            &db,
            &ExportRequest {
                dataset_id: dataset.id,
                dest_dir: tmp.path().to_path_buf(),
                caption_order: compose::CaptionOrder::ProseFirst,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("nothing to export"), "{err}");
    }

    #[tokio::test]
    async fn export_dataset_writes_numbered_media_and_composed_caption_pairs() {
        let db = Database::connect_in_memory().await.unwrap();
        let (job_id, dataset) = frames_dataset(&db).await;
        db.datasets()
            .set_trigger_word(&dataset.id, "ghibli_xy")
            .await
            .unwrap();
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();

        let mut ids = Vec::new();
        for i in 0..3 {
            let id = insert_frame(&db, &job_id, &dataset.id, src.path(), i, "").await;
            db.dataset_frames()
                .set_caption(&id, &format!("caption {i}"), "florence2")
                .await
                .unwrap();
            ids.push(id);
        }
        // Exclude the middle frame; a rejected one is never exported either.
        db.dataset_frames()
            .set_excluded(&ids[1], true)
            .await
            .unwrap();
        insert_frame(&db, &job_id, &dataset.id, src.path(), 3, "blur").await;

        let concept = db
            .concepts()
            .create(crate::db::NewConcept {
                dataset_id: dataset.id.clone(),
                name: "Kenji".into(),
                token: "kenji_xy".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        db.concepts()
            .assign(&concept.id, &[ids[0].clone()])
            .await
            .unwrap();

        let summary = export_dataset(
            &db,
            &ExportRequest {
                dataset_id: dataset.id.clone(),
                dest_dir: dest.path().to_path_buf(),
                caption_order: compose::CaptionOrder::ProseFirst,
            },
        )
        .await
        .unwrap();
        assert_eq!(summary.exported, 2);
        assert!(dest.path().join("0001.png").is_file());
        assert_eq!(
            std::fs::read_to_string(dest.path().join("0001.txt")).unwrap(),
            "caption 0, ghibli_xy, kenji_xy"
        );
        assert!(dest.path().join("0002.png").is_file());
        assert_eq!(
            std::fs::read_to_string(dest.path().join("0002.txt")).unwrap(),
            "caption 2, ghibli_xy"
        );
        assert_eq!(
            db.datasets()
                .get(&dataset.id)
                .await
                .unwrap()
                .unwrap()
                .export_dir
                .as_deref(),
            Some(dest.path().to_string_lossy().as_ref())
        );

        // The same dataset, the other training profile's caption order.
        let tags_first = tempfile::tempdir().unwrap();
        export_dataset(
            &db,
            &ExportRequest {
                dataset_id: dataset.id.clone(),
                dest_dir: tags_first.path().to_path_buf(),
                caption_order: compose::CaptionOrder::TagsFirst,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(tags_first.path().join("0001.txt")).unwrap(),
            "ghibli_xy, kenji_xy, caption 0"
        );
    }

    #[tokio::test]
    async fn export_dataset_for_job_finds_the_dataset_by_prep_job() {
        let db = Database::connect_in_memory().await.unwrap();
        let (job_id, dataset) = frames_dataset(&db).await;
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        insert_frame(&db, &job_id, &dataset.id, src.path(), 0, "").await;

        let summary = export_dataset_for_job(&db, &job_id, dest.path())
            .await
            .unwrap();
        assert_eq!(summary.exported, 1);

        let orphan = new_job(&db).await;
        let err = export_dataset_for_job(&db, &orphan, dest.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no dataset"), "{err}");
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

    #[tokio::test]
    async fn run_without_a_captioner_creates_a_dataset_and_keeps_and_rejects_frames_with_reasons() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("dataset_prep"))
            .await
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let tag_dir = root.path().join("MyStyle");
        std::fs::create_dir_all(&tag_dir).unwrap();
        // A sharp image (kept) and a black one (rejected as dead).
        let sharp = image::DynamicImage::ImageLuma8({
            let mut b = image::GrayImage::new(32, 32);
            for y in 0..32 {
                for x in 0..32 {
                    b.put_pixel(
                        x,
                        y,
                        image::Luma([if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 }]),
                    );
                }
            }
            b
        });
        sharp.save(tag_dir.join("a.png")).unwrap();
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(32, 32, image::Luma([2])))
            .save(tag_dir.join("b.png"))
            .unwrap();

        let req = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": root.path().to_string_lossy() }),
        )
        .unwrap();
        let work = tempfile::tempdir().unwrap();
        let (_tx, rx) = watch::channel(false);
        let outcome = run(&db, &vision, work.path(), &job.id, req, rx)
            .await
            .unwrap();
        let DatasetPrepOutcome::Done(done) = outcome else {
            panic!("expected Done")
        };
        assert_eq!(done.frame_count, 1, "only the sharp frame is kept");

        let datasets = db.datasets().list().await.unwrap();
        assert_eq!(datasets.len(), 1);
        assert_eq!(
            datasets[0].name,
            root.path().file_name().unwrap().to_string_lossy()
        );
        assert_eq!(datasets[0].prep_job_id.as_deref(), Some(job.id.as_str()));

        let frames = db
            .dataset_frames()
            .list_for_dataset(&datasets[0].id)
            .await
            .unwrap();
        assert_eq!(frames.len(), 2, "rejected frames are stored too");
        let reasons: Vec<&str> = frames.iter().map(|f| f.rejection_reason.as_str()).collect();
        assert!(reasons.contains(&""), "{reasons:?}");
        assert!(reasons.contains(&"black"), "{reasons:?}");
        assert!(
            frames.iter().all(|f| f.caption.is_empty()),
            "no captioner -> no captions"
        );
    }

    /// Mirrors `filter.rs`'s own fixtures so the verdicts below rest on the
    /// same pictures its unit tests pin.
    fn sharp_checkerboard(size: u32, phase: u32) -> image::DynamicImage {
        let mut buf = image::GrayImage::new(size, size);
        for y in 0..size {
            for x in 0..size {
                let v = if (x / 4 + y / 4 + phase) % 2 == 0 {
                    255
                } else {
                    0
                };
                buf.put_pixel(x, y, image::Luma([v]));
            }
        }
        image::DynamicImage::ImageLuma8(buf)
    }

    fn flat_gray(size: u32, value: u8) -> image::DynamicImage {
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
            size,
            size,
            image::Luma([value]),
        ))
    }

    fn candidate(path: PathBuf) -> FrameCandidate {
        FrameCandidate {
            source: PathBuf::from("E:\\Data\\Ghibli\\clip.mp4"),
            tag: "Ghibli".into(),
            timestamp_secs: Some(0.0),
            path,
        }
    }

    /// [sharp A, sharp A again, black, flat mid-gray, distinct sharp B].
    fn mixed_group(dir: &Path) -> CandidateGroup {
        let files: [(&str, image::DynamicImage); 5] = [
            ("a.png", sharp_checkerboard(32, 0)),
            ("a-dup.png", sharp_checkerboard(32, 0)),
            ("black.png", flat_gray(32, 2)),
            ("blurry.png", flat_gray(32, 128)),
            ("b.png", sharp_checkerboard(64, 1)),
        ];
        let mut frames = Vec::new();
        for (name, img) in files {
            let p = dir.join(name);
            img.save(&p).unwrap();
            frames.push(candidate(p));
        }
        CandidateGroup {
            source: PathBuf::from("E:\\Data\\Ghibli\\clip.mp4"),
            frames,
        }
    }

    #[test]
    fn filter_groups_records_the_first_matching_reason_per_frame() {
        use filter::RejectionReason as R;
        let tmp = tempfile::tempdir().unwrap();
        let group = mixed_group(tmp.path());
        // Pin the fixture's own distances so a verdict change can never be
        // blamed on an accidentally-similar picture.
        let hashes: Vec<_> = group
            .frames
            .iter()
            .map(|c| filter::phash_of(&c.path).unwrap())
            .collect();
        assert_eq!(hashes[0].dist(&hashes[1]), 0, "the duplicate is identical");
        assert!(
            hashes[0].dist(&hashes[4]) > DEFAULT_PHASH_MAX_DISTANCE,
            "B must be distinct: {}",
            hashes[0].dist(&hashes[4])
        );

        let req = DatasetPrepRequest::from_params(&serde_json::json!({ "root": "x" })).unwrap();
        let (_tx, rx) = watch::channel(false);
        let (judged, extracted, kept) = filter_groups(&rx, vec![group], &req).unwrap().unwrap();
        assert_eq!((extracted, kept), (5, 2));
        let verdicts: Vec<Option<R>> = judged[0].frames.iter().map(|(_, v)| *v).collect();
        assert_eq!(
            verdicts,
            vec![
                None,
                Some(R::Duplicate),
                Some(R::Black),
                Some(R::Blur),
                None
            ]
        );
    }

    #[test]
    fn filter_groups_caps_the_survivors_by_diversity() {
        let tmp = tempfile::tempdir().unwrap();
        let group = mixed_group(tmp.path());
        let req = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": "x", "max_frames_per_clip": 1 }),
        )
        .unwrap();
        let (_tx, rx) = watch::channel(false);
        let (judged, _, kept) = filter_groups(&rx, vec![group], &req).unwrap().unwrap();
        assert_eq!(kept, 1);
        let capped = judged[0]
            .frames
            .iter()
            .filter(|(_, v)| *v == Some(filter::RejectionReason::Cap))
            .count();
        assert_eq!(capped, 1, "the second survivor is dropped by the cap");
    }

    #[test]
    fn clip_is_usable_needs_a_duration_at_or_above_the_minimum() {
        assert!(
            !clip_is_usable(None, 2.0),
            "undecodable / no duration is never usable"
        );
        assert!(!clip_is_usable(Some(1.0), 2.0));
        assert!(clip_is_usable(Some(2.0), 2.0), "the bound is inclusive");
        assert!(clip_is_usable(Some(30.0), 2.0));
        assert!(clip_is_usable(Some(0.0), 0.0), "a zero minimum accepts all");
    }
}
