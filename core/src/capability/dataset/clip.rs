//! Clip mode (spec 3): the half of the pipeline that does *not* extract
//! frames. Each source video stays one object — one `dataset_frames` row
//! carrying its duration and a preview still — and the curator sets an
//! in/out range that [`super::export`] trims to at export time.

use std::collections::BTreeMap;
use std::path::Path;

use tokio::sync::watch;

use crate::db::{Database, EventLevel, NewDatasetFrame};
use crate::Result;

use super::pipeline::{cancelled, video_stem, DatasetPrepDone, DatasetPrepOutcome};
use super::request::DatasetPrepRequest;
use super::{dataset_err, extract, filter, ingest};

/// Clip mode's one judgement call: a video whose duration ffprobe could not
/// read (undecodable) or that is shorter than the run's minimum is recorded
/// as [`filter::RejectionReason::Unusable`] instead of kept. Pure, so it is
/// testable without ffmpeg installed.
fn clip_is_usable(duration: Option<f64>, min_secs: f64) -> bool {
    duration.is_some_and(|d| d >= min_secs)
}

/// What clip mode decided about one video, and what (if anything) the job log
/// should say about it.
#[derive(Debug, PartialEq)]
struct ClipVerdict {
    usable: bool,
    /// `Some` only when something went wrong that the curator should see; a
    /// clip that is simply too short is an expected outcome, not a warning.
    warning: Option<String>,
}

/// Fold the two things that can disqualify one clip into a single verdict:
/// it is too short (or undecodable), or its preview still could not be
/// written. The second is deliberately *not* fatal to the run — one clip
/// ffmpeg cannot seek into should not throw away every other clip's work —
/// so it comes back as a warning to log alongside the `Unusable` row.
fn clip_verdict(duration: Option<f64>, min_secs: f64, preview: Result<()>) -> ClipVerdict {
    if !clip_is_usable(duration, min_secs) {
        return ClipVerdict {
            usable: false,
            warning: None,
        };
    }
    match preview {
        Ok(()) => ClipVerdict {
            usable: true,
            warning: None,
        },
        Err(e) => ClipVerdict {
            usable: false,
            warning: Some(format!("no preview still \u{2014} {e}")),
        },
    }
}

/// Clip mode (spec 3): no frame extraction at all — each source *video*
/// becomes one dataset row carrying its duration plus a preview still, ready
/// for the curator to set an in/out range that export trims to. Plain images
/// are skipped: there is no clip to trim.
pub(super) async fn run_clip_mode(
    db: &Database,
    work_dir: &Path,
    job_id: &str,
    dataset_id: &str,
    items: &[ingest::IngestItem],
    req: &DatasetPrepRequest,
    cancel: &watch::Receiver<bool>,
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
        if cancelled(cancel).await {
            return Ok(DatasetPrepOutcome::Cancelled);
        }
        let duration = extract::probe_duration_secs(&ffprobe, &item.path).await?;
        let preview = preview_dir
            .join(&item.tag)
            .join(format!("{}.png", video_stem(&item.path)));
        let preview_result = if clip_is_usable(duration, req.min_clip_secs) {
            // One second in, or the very start for a clip barely that long —
            // the first frame of a cut is often a fade.
            let at = duration.unwrap_or(0.0).min(1.0);
            extract::extract_preview_still(&ffmpeg, &item.path, &preview, at).await
        } else {
            Ok(())
        };
        let ClipVerdict { usable, warning } =
            clip_verdict(duration, req.min_clip_secs, preview_result);
        if let Some(warning) = warning {
            db.jobs()
                .append_event(
                    job_id,
                    EventLevel::Warn,
                    &format!("{}: {warning}", item.path.display()),
                )
                .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_verdict_rejects_short_clips_and_survives_a_preview_failure() {
        let short = clip_verdict(Some(1.0), 2.0, Ok(()));
        assert!(!short.usable);
        assert_eq!(short.warning, None, "too short is not worth a warning");

        let good = clip_verdict(Some(30.0), 2.0, Ok(()));
        assert!(good.usable);
        assert_eq!(good.warning, None);

        let no_preview = clip_verdict(Some(30.0), 2.0, Err(dataset_err("ffmpeg preview failed")));
        assert!(
            !no_preview.usable,
            "a clip without a preview still cannot be curated"
        );
        let warning = no_preview.warning.unwrap();
        assert!(warning.contains("ffmpeg preview failed"), "{warning}");
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
