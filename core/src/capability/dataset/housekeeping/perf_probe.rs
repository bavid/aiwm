//! Timing probe for building the deletion guard against a crowded outputs
//! folder. Run on demand:
//! `cargo test -p aiwm-core --release --lib guard_build_timing -- --ignored --nocapture`.

use std::path::{Path, PathBuf};

use super::guard::{Guard, Snapshot};
use crate::db::{Dataset, DatasetFrame, DatasetMode};

const FOREIGN_DATASETS: usize = 20;
const FRAMES_PER_DATASET: usize = 5_000;
const FOLDERS_PER_DATASET: usize = 10;
const OWN_FRAMES: usize = 5_000;

fn dataset(id: &str, prep_job_id: &str, source_root: &Path) -> Dataset {
    Dataset {
        id: id.into(),
        name: id.into(),
        mode: DatasetMode::Frames,
        source_root: source_root.to_string_lossy().into_owned(),
        trigger_word: String::new(),
        prep_job_id: Some(prep_job_id.into()),
        export_dir: None,
        work_dir: None,
        created_at: String::new(),
    }
}

/// `count` empty frame files spread over `folders` clip folders of one job,
/// each with its own source video; returns `(frame_path, source_path)`.
fn frames_on_disk(
    datasets_root: &Path,
    job: &str,
    src: &Path,
    count: usize,
    folders: usize,
) -> Vec<(String, String)> {
    std::fs::create_dir_all(src).unwrap();
    (0..count)
        .map(|n| {
            let k = n % folders;
            let dir = datasets_root
                .join(job)
                .join("raw")
                .join("T")
                .join(format!("v{k}"));
            if n < folders {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(src.join(format!("v{k}.mp4")), b"").unwrap();
            }
            let frame: PathBuf = dir.join(format!("frame_{n:06}.png"));
            std::fs::write(&frame, b"").unwrap();
            (
                frame.to_string_lossy().into_owned(),
                src.join(format!("v{k}.mp4")).to_string_lossy().into_owned(),
            )
        })
        .collect()
}

#[test]
#[ignore = "timing probe; run with --ignored --nocapture"]
fn guard_build_timing() {
    let tmp = tempfile::tempdir().unwrap();
    let outputs = tmp.path().join("outputs");
    let datasets_root = outputs.join("datasets");

    let started = std::time::Instant::now();
    let mut others = Vec::new();
    let mut foreign_frames = Vec::new();
    for i in 0..FOREIGN_DATASETS {
        let job = format!("job-{i:02}");
        let src = tmp.path().join(format!("src-{i:02}"));
        others.push(dataset(&format!("ds-{i:02}"), &job, &src));
        foreign_frames.extend(frames_on_disk(
            &datasets_root,
            &job,
            &src,
            FRAMES_PER_DATASET,
            FOLDERS_PER_DATASET,
        ));
    }
    let own_src = tmp.path().join("src-own");
    let own_paths = frames_on_disk(&datasets_root, "job-own", &own_src, OWN_FRAMES, 20);
    let frames: Vec<DatasetFrame> = own_paths
        .iter()
        .enumerate()
        .map(|(n, (frame, source))| DatasetFrame {
            id: format!("f{n}"),
            job_id: None,
            dataset_id: Some("own".into()),
            tag: "T".into(),
            source_path: source.clone(),
            frame_path: frame.clone(),
            timestamp_secs: None,
            caption: String::new(),
            caption_engine: String::new(),
            excluded: false,
            rejection_reason: String::new(),
            duration_secs: None,
            clip_start_secs: None,
            clip_end_secs: None,
            created_at: String::new(),
        })
        .collect();
    println!(
        "fixture: {} foreign frame rows in {} folders + {} own frames, written in {:?}",
        foreign_frames.len(),
        FOREIGN_DATASETS * FOLDERS_PER_DATASET,
        frames.len(),
        started.elapsed()
    );
    let snap = Snapshot {
        dataset: dataset("own", "job-own", &own_src),
        frames,
        others,
        foreign_frames,
    };

    for (label, staying) in [
        ("delete dataset (nothing stays)", Vec::new()),
        (
            "usage / cleanup (all own frames stay)",
            snap.frames.iter().map(|f| f.frame_path.as_str()).collect(),
        ),
    ] {
        let t = std::time::Instant::now();
        let guard = Guard::build(
            &super::DataRoots {
                outputs: outputs.clone(),
                datasets: datasets_root.clone(),
                models: tmp.path().join("models"),
            },
            &snap,
        )
        .keeping(&staying);
        println!(
            "Guard::build, {label}: {:?} (walkable: {})",
            t.elapsed(),
            guard.walkable
        );
    }
}
