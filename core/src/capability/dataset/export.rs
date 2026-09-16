//! Export: turn one curated dataset into the `NNNN.<ext>` + `NNNN.txt`
//! sidecar-file pairs kohya-ss/sd-scripts and similar LoRA trainers expect.
//!
//! The caption written here is *composed* (spec 3A/3D) from the dataset's
//! trigger word, the frame's assigned concepts and its own auto/hand
//! caption — the stored `dataset_frames.caption` is only one ingredient, and
//! is never overwritten by an export.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::{Database, DatasetFrame, DatasetMode};
use crate::Result;

use super::{captioner, compose, dataset_err, extract};

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
        write_media(Path::new(src), &media_dest, frame, ffmpeg.as_deref()).await?;

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

/// Put one item's media at `dest`: a stream-copy trim when the curator set a
/// clip range, a plain copy otherwise. `ffmpeg` is `Some` only in clips mode.
///
/// Refusing to fall back to a whole-clip copy when ffmpeg is missing is
/// deliberate: that would hand the trainer exactly the footage the curator
/// cut away, silently.
async fn write_media(
    src: &Path,
    dest: &Path,
    frame: &DatasetFrame,
    ffmpeg: Option<&Path>,
) -> Result<()> {
    let has_range = frame.clip_start_secs.is_some() || frame.clip_end_secs.is_some();
    match (has_range, ffmpeg) {
        (true, Some(ffmpeg)) => {
            extract::trim_clip(
                ffmpeg,
                src,
                dest,
                frame.clip_start_secs,
                frame.clip_end_secs,
            )
            .await
        }
        (true, None) => Err(dataset_err(
            "ffmpeg was not found on PATH \u{2014} needed to trim clips with a start/end range",
        )),
        (false, _) => tokio::fs::copy(src, dest).await.map(|_| ()).map_err(|e| {
            dataset_err(format!(
                "copy {} \u{2192} {}: {e}",
                src.display(),
                dest.display()
            ))
        }),
    }
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
    use super::super::testutil::{frames_dataset, insert_frame, new_job};
    use super::*;

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
}
