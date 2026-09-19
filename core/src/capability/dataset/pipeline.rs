//! Frames mode end to end: walk the root, extract stills, judge every
//! candidate, file them all under one dataset, and (optionally) caption the
//! keepers. [`run`] is the entry point the job engine calls; clip mode
//! branches off into [`super::clip`] right after the dataset row exists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::sync::watch;

use crate::db::{Database, DatasetFrame, DatasetMode, EventLevel, NewDatasetFrame};
use crate::runtime::VisionAdapter;
use crate::Result;

use super::captioner::Captioner;
use super::request::DatasetPrepRequest;
use super::{caption, captioner, clip, dataset_err, extract, filter, ingest};

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

pub(super) fn video_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip")
        .to_string()
}

pub(super) async fn cancelled(cancel: &watch::Receiver<bool>) -> bool {
    *cancel.borrow()
}

/// Delete `dataset_id` again when nothing was ever filed under it — the
/// cleanup half of [`run`]'s "never an empty dataset" rule. Best-effort on
/// purpose: it runs while an error is already on its way out, so a failure
/// here is logged rather than allowed to mask the real one.
async fn discard_empty_dataset(db: &Database, dataset_id: &str) {
    match db.dataset_frames().list_for_dataset(dataset_id).await {
        Ok(frames) if frames.is_empty() => {
            if let Err(e) = db.datasets().delete(dataset_id).await {
                tracing::warn!(dataset_id, error = %e, "could not discard the empty dataset");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(dataset_id, error = %e, "could not check the dataset for frames"),
    }
}

/// Run the whole pipeline for `req`, writing extracted frames under
/// `work_dir/<job_id>/raw/` and persisting kept, captioned frames to the
/// `dataset_frames` table. The vision runtime's model is already loaded by
/// the time this runs (the job engine's generic Target/scheduler flow, same
/// as every other capability) — this function only ever calls
/// `vision.client()` to talk to the already-running sidecar, never
/// `load_model` itself.
///
/// A dataset is never left behind without frames: a cancelled or failed run
/// keeps whatever frames and captions were already written — the job's fate
/// stays visible through `prep_job_id`, and a partial dataset is safe to
/// curate and export — but one that never received a single frame is deleted
/// again rather than left as an empty shell.
///
/// `store_root` is the model store (`config.store_path`): Florence-2 and
/// Qwen2.5-VL only ever load from its pinned `vision/…` subfolders, and only
/// after the load-time integrity check (`model::integrity`) passed here —
/// once per run, before the first `caption_frame` / `caption_frame_pair` is
/// sent (the sidecar then keeps the loaded engine for the rest of the run).
pub async fn run(
    db: &Database,
    vision: &VisionAdapter,
    store_root: &Path,
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
        Some(c) => Some(caption::resolve_captioner_dir(db, store_root, c).await?),
        None => None,
    };

    let escalate = req.escalate && captioner.is_some_and(|c| c.supports_escalation);
    let qwen_dir = if escalate {
        match caption::resolve_qwen_vl_dir(db, store_root).await {
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
    // Qwen loads lazily in the sidecar at the first escalation, which can
    // come long after the check above -- the gate re-verifies the folder
    // right before that first call (once per run).
    let qwen_gate = caption::QwenGate::new(store_root);

    let items = ingest::walk_dataset_root(&req.root)?;
    if items.is_empty() {
        return Err(dataset_err(format!(
            "no videos or images found under {} \u{2014} put .mp4/.png/.jpg/.jpeg/.webp files \
             in the folder itself or in tag subfolders",
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

    // Everything past this point has a `datasets` row to clean up on the way
    // out, so it lives in one block whose single result is checked below.
    let outcome: Result<DatasetPrepOutcome> = async {
        let tag_count: std::collections::BTreeSet<&String> =
            items.iter().map(|i| &i.tag).collect();
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
            return clip::run_clip_mode(db, work_dir, job_id, &dataset.id, &items, &req, &cancel).await;
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
        let mut kept_records: Vec<DatasetFrame> = Vec::with_capacity(group.frames.len());
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
            let ctx = CaptionContext {
                vision,
                captioner: c,
                model_dir: dir,
                qwen_gate: &qwen_gate,
                job_id,
                escalate,
                req: &req,
            };
            caption_group(db, &ctx, &kept_records, &cancel).await?;
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
    .await;

    // Cancelling counts the same as failing here: neither is a reason to keep
    // a dataset nobody can curate.
    if !matches!(outcome, Ok(DatasetPrepOutcome::Done(_))) {
        discard_empty_dataset(db, &dataset.id).await;
    }
    outcome
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
/// Everything captioning needs that is fixed for the whole run — resolved
/// once in [`run`] and handed to [`caption_group`] as one value instead of
/// six positional arguments.
struct CaptionContext<'a> {
    vision: &'a VisionAdapter,
    captioner: &'a Captioner,
    model_dir: &'a Path,
    /// Re-verifies the Qwen2.5-VL folder before the first escalation call
    /// and hands out the verified directory (or `None` once disabled).
    qwen_gate: &'a caption::QwenGate,
    /// For the gate's "escalation disabled" warning event.
    job_id: &'a str,
    /// Already implies `captioner.supports_escalation`.
    escalate: bool,
    req: &'a DatasetPrepRequest,
}

async fn caption_group(
    db: &Database,
    ctx: &CaptionContext<'_>,
    records: &[DatasetFrame],
    cancel: &watch::Receiver<bool>,
) -> Result<()> {
    let CaptionContext {
        vision,
        captioner: c,
        model_dir,
        qwen_gate,
        job_id,
        escalate,
        req,
    } = *ctx;
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
            let qwen_dir = match neighbor_idx {
                Some(_) => qwen_gate.dir_for_escalation(db, job_id).await?,
                None => None,
            };
            if let (Some(neighbor_idx), Some(qwen_dir)) = (neighbor_idx, qwen_dir) {
                let neighbor_path = Path::new(&records[neighbor_idx].frame_path);
                if let Ok(recap) =
                    caption::caption_frame_pair(vision, &qwen_dir, frame_path, neighbor_path).await
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

#[cfg(test)]
mod tests {
    use super::super::testutil::{frames_dataset, insert_frame, new_job};
    use super::*;
    use crate::capability::dataset::DEFAULT_PHASH_MAX_DISTANCE;

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

        let err = run(&db, &vision, tmp.path(), tmp.path(), "job-1", req, rx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
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
        let outcome = run(&db, &vision, work.path(), work.path(), &job.id, req, rx)
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
    #[tokio::test]
    async fn run_accepts_images_placed_directly_in_the_root_under_the_root_name_tag() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let job = new_job(&db).await;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Test");
        std::fs::create_dir_all(&root).unwrap();
        sharp_checkerboard(32, 0).save(root.join("a.png")).unwrap();

        let req =
            DatasetPrepRequest::from_params(&serde_json::json!({ "root": root.to_string_lossy() }))
                .unwrap();
        let work = tempfile::tempdir().unwrap();
        let (_tx, rx) = watch::channel(false);
        let outcome = run(&db, &vision, work.path(), work.path(), &job, req, rx)
            .await
            .unwrap();
        let DatasetPrepOutcome::Done(done) = outcome else {
            panic!("expected Done")
        };
        assert_eq!(done.frame_count, 1);

        let datasets = db.datasets().list().await.unwrap();
        let frames = db
            .dataset_frames()
            .list_for_dataset(&datasets[0].id)
            .await
            .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].tag, "Test");
    }

    #[tokio::test]
    async fn run_on_an_empty_root_explains_where_media_may_live() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let job = new_job(&db).await;
        let root = tempfile::tempdir().unwrap();
        let req = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": root.path().to_string_lossy() }),
        )
        .unwrap();
        let work = tempfile::tempdir().unwrap();
        let (_tx, rx) = watch::channel(false);
        let err = run(&db, &vision, work.path(), work.path(), &job, req, rx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("no videos or images found under"), "{err}");
        assert!(
            err.contains("in the folder itself or in tag subfolders"),
            "{err}"
        );
        assert!(db.datasets().list().await.unwrap().is_empty());
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

        let req = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": std::env::temp_dir().to_string_lossy() }),
        )
        .unwrap();
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
            &serde_json::json!({ "root": std::env::temp_dir().to_string_lossy(), "max_frames_per_clip": 1 }),
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
    #[tokio::test]
    async fn discard_empty_dataset_only_deletes_one_that_never_got_a_frame() {
        let db = Database::connect_in_memory().await.unwrap();
        let src = tempfile::tempdir().unwrap();

        let (_job, empty) = frames_dataset(&db).await;
        discard_empty_dataset(&db, &empty.id).await;
        assert!(db.datasets().get(&empty.id).await.unwrap().is_none());

        let (job_id, with_frames) = frames_dataset(&db).await;
        insert_frame(&db, &job_id, &with_frames.id, src.path(), 0, "").await;
        discard_empty_dataset(&db, &with_frames.id).await;
        assert!(
            db.datasets().get(&with_frames.id).await.unwrap().is_some(),
            "a failed run that already stored frames keeps them"
        );
    }

    #[tokio::test]
    async fn a_failed_run_never_leaves_an_empty_dataset_behind() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let job = new_job(&db).await;
        let work = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let tag_dir = root.path().join("MyStyle");
        std::fs::create_dir_all(&tag_dir).unwrap();
        // A file the filter stage cannot decode: the run fails *after* the
        // dataset row exists but before a single frame is stored.
        std::fs::write(tag_dir.join("broken.png"), b"not a png").unwrap();

        // Failing *before* the row is created leaves nothing behind either.
        let needs_model = DatasetPrepRequest::from_params(&serde_json::json!({
            "root": root.path().to_string_lossy(), "captioner": "florence2"
        }))
        .unwrap();
        let (_tx, rx) = watch::channel(false);
        let err = run(
            &db,
            &vision,
            work.path(),
            work.path(),
            &job,
            needs_model,
            rx,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Florence-2"), "{err}");
        assert!(db.datasets().list().await.unwrap().is_empty());

        let req = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": root.path().to_string_lossy() }),
        )
        .unwrap();
        let (_tx, rx) = watch::channel(false);
        let err = run(&db, &vision, work.path(), work.path(), &job, req, rx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("dead-frame check"), "{err}");
        assert!(
            db.datasets().list().await.unwrap().is_empty(),
            "the dataset row was rolled back"
        );
    }

    #[tokio::test]
    async fn a_run_cancelled_before_its_first_frame_leaves_no_dataset_behind() {
        let db = Database::connect_in_memory().await.unwrap();
        let vision = VisionAdapter::new();
        let job = new_job(&db).await;
        let work = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let tag_dir = root.path().join("MyStyle");
        std::fs::create_dir_all(&tag_dir).unwrap();
        sharp_checkerboard(32, 0)
            .save(tag_dir.join("a.png"))
            .unwrap();

        // Cancelled before the first candidate is even extracted: the run
        // comes to rest cleanly, but nothing was ever filed under the dataset.
        let (tx, rx) = watch::channel(false);
        tx.send(true).unwrap();
        let req = DatasetPrepRequest::from_params(
            &serde_json::json!({ "root": root.path().to_string_lossy() }),
        )
        .unwrap();
        let outcome = run(&db, &vision, work.path(), work.path(), &job, req, rx)
            .await
            .unwrap();
        assert!(matches!(outcome, DatasetPrepOutcome::Cancelled));
        assert!(
            db.datasets().list().await.unwrap().is_empty(),
            "a frameless cancelled run is discarded like a failed one"
        );
    }
}
