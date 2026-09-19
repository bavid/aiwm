//! Fixtures shared by the dataset submodules' own unit tests: the smallest
//! job + dataset + frame rows that satisfy the schema's foreign keys.
//! `dataset_frames.job_id` really does reference `jobs`, so a frame always
//! needs a job behind it — which is exactly the kind of setup worth writing
//! once rather than in every test module.

use std::path::Path;

use crate::db::{Database, Dataset, DatasetMode, NewDataset, NewDatasetFrame, NewJob};

pub(super) async fn new_job(db: &Database) -> String {
    db.jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap()
        .id
}

/// A `dataset_prep` job plus the frames-mode dataset it produced.
pub(super) async fn frames_dataset(db: &Database) -> (String, Dataset) {
    let job_id = new_job(db).await;
    let dataset = db
        .datasets()
        .create(NewDataset {
            name: "Ghibli".into(),
            mode: DatasetMode::Frames,
            source_root: "E:\\Data\\Ghibli".into(),
            prep_job_id: Some(job_id.clone()),
            work_dir: None,
        })
        .await
        .unwrap();
    (job_id, dataset)
}

/// One frame row with a real file on disk behind it.
pub(super) async fn insert_frame(
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
