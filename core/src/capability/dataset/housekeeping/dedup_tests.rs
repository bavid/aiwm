//! The dataset-wide duplicate search against real PNGs in a temp dataset.

use std::path::{Path, PathBuf};

use image::{GrayImage, Luma};

use super::tests::{fixture, fixture_with_mode};
use super::*;
use crate::db::DatasetMode;

// --- global dedup -------------------------------------------------------------

/// 64x64 with 16px blocks: low-frequency structure a perceptual hash sees.
fn blocks() -> GrayImage {
    GrayImage::from_fn(64, 64, |x, y| {
        Luma([if (x / 16 + y / 16) % 2 == 0 { 230 } else { 20 }])
    })
}

fn stripes() -> GrayImage {
    GrayImage::from_fn(64, 64, |x, _| {
        Luma([if (x / 8) % 2 == 0 { 240 } else { 10 }])
    })
}

fn gradient() -> GrayImage {
    GrayImage::from_fn(64, 64, |x, y| Luma([((x + y) * 2).min(255) as u8]))
}

fn save(dir: &Path, name: &str, img: &GrayImage) -> PathBuf {
    let path = dir.join(name);
    img.save(&path).unwrap();
    path
}

#[tokio::test]
async fn dedup_groups_across_sources_and_keeps_the_sharpest_frame() {
    let fx = fixture().await;
    let dir_a = fx.clip_dir.clone();
    let dir_b = fx.work_root.join("raw").join("Tag").join("other");
    std::fs::create_dir_all(&dir_b).unwrap();
    let source_b = fx.tmp.path().join("src").join("other.mp4");
    let blurred = imageproc::filter::gaussian_blur_f32(&blocks(), 1.5);

    // The blurred copies land first, so insertion order cannot pick the keeper.
    let a_blur = fx
        .row(&save(&dir_a, "a_blur.png", &blurred), &fx.source, "", false)
        .await;
    let a_sharp = fx
        .row(
            &save(&dir_a, "a_sharp.png", &blocks()),
            &fx.source,
            "",
            false,
        )
        .await;
    let a_other = fx
        .row(
            &save(&dir_a, "a_other.png", &stripes()),
            &fx.source,
            "",
            false,
        )
        .await;
    let b_blur = fx
        .row(&save(&dir_b, "b_blur.png", &blurred), &source_b, "", false)
        .await;
    let b_other = fx
        .row(
            &save(&dir_b, "b_other.png", &gradient()),
            &source_b,
            "",
            false,
        )
        .await;
    // Already discarded frames are not looked at.
    let excluded = fx
        .row(
            &save(&dir_b, "b_excluded.png", &blocks()),
            &source_b,
            "",
            true,
        )
        .await;

    // Fixture sanity: the blurred copy really is a near-duplicate, the other
    // scenes really are different pictures.
    let (h_sharp, s_sharp) = filter::phash_and_sharpness(&dir_a.join("a_sharp.png")).unwrap();
    let (h_blur, s_blur) = filter::phash_and_sharpness(&dir_a.join("a_blur.png")).unwrap();
    let (h_stripes, _) = filter::phash_and_sharpness(&dir_a.join("a_other.png")).unwrap();
    assert!(h_sharp.dist(&h_blur) <= DEFAULT_DEDUP_THRESHOLD);
    assert!(s_sharp > s_blur);
    assert!(h_sharp.dist(&h_stripes) > MAX_DEDUP_THRESHOLD);

    let s = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    assert_eq!(s.threshold, DEFAULT_DEDUP_THRESHOLD);
    assert_eq!(s.scanned, 5);
    assert_eq!(s.groups, 1);
    assert_eq!(s.marked, 2);
    assert_eq!(s.unreadable, 0);

    let reason = |id: String| {
        let db = &fx.db;
        async move {
            db.dataset_frames()
                .get(&id)
                .await
                .unwrap()
                .unwrap()
                .rejection_reason
        }
    };
    assert_eq!(reason(a_sharp).await, "");
    assert_eq!(reason(a_other).await, "");
    assert_eq!(reason(b_other).await, "");
    assert_eq!(reason(excluded).await, "");
    assert_eq!(reason(a_blur).await, "duplicate_global");
    assert_eq!(reason(b_blur).await, "duplicate_global");

    // Deterministic and idempotent: a second run finds nothing new.
    let again = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    assert_eq!((again.scanned, again.marked), (3, 0));
}

#[tokio::test]
async fn dedup_breaks_a_sharpness_tie_toward_the_earlier_frame() {
    let fx = fixture().await;
    let first = fx
        .row(
            &save(&fx.clip_dir, "1.png", &blocks()),
            &fx.source,
            "",
            false,
        )
        .await;
    let second = fx
        .row(
            &save(&fx.clip_dir, "2.png", &blocks()),
            &fx.source,
            "",
            false,
        )
        .await;
    let s = dedup(&fx.db, &fx.dataset.id, Some(0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((s.groups, s.marked), (1, 1));
    let repo = fx.db.dataset_frames();
    assert_eq!(
        repo.get(&first).await.unwrap().unwrap().rejection_reason,
        ""
    );
    assert_eq!(
        repo.get(&second).await.unwrap().unwrap().rejection_reason,
        "duplicate_global"
    );
}

#[tokio::test]
async fn dedup_counts_unreadable_frames_instead_of_failing() {
    let fx = fixture().await;
    fx.frame("broken.png", 10, "", false).await;
    fx.row(
        &save(&fx.clip_dir, "ok.png", &blocks()),
        &fx.source,
        "",
        false,
    )
    .await;
    let s = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    assert_eq!((s.scanned, s.unreadable, s.marked), (2, 1, 0));
}

#[test]
fn dedup_threshold_defaults_to_the_filter_constant_and_clamps_to_16() {
    assert_eq!(dedup_threshold(None), filter::DEFAULT_PHASH_MAX_DISTANCE);
    assert_eq!(dedup_threshold(Some(0)), 0);
    assert_eq!(dedup_threshold(Some(16)), 16);
    assert_eq!(dedup_threshold(Some(64)), 16);
}

#[tokio::test]
async fn dedup_reports_the_clamped_threshold() {
    let fx = fixture().await;
    let s = dedup(&fx.db, &fx.dataset.id, Some(99))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.threshold, MAX_DEDUP_THRESHOLD);
}

#[tokio::test]
async fn dedup_refuses_a_clips_dataset_and_ignores_unknown_ids() {
    let fx = fixture_with_mode(DatasetMode::Clips).await;
    let err = dedup(&fx.db, &fx.dataset.id, None).await.unwrap_err();
    assert!(matches!(err, CoreError::Config(_)), "{err}");
    assert!(dedup(&fx.db, "nope", None).await.unwrap().is_none());
}

/// Timing probe, run on demand:
/// `cargo test -p aiwm-core --lib dedup_timing -- --ignored --nocapture`.
/// 1000 distinct 64x64 frames, then 50 1280x720 frames to extrapolate to
/// real extracted stills.
#[tokio::test]
#[ignore]
async fn dedup_timing() {
    let fx = fixture().await;
    for i in 0..1000u32 {
        let img = GrayImage::from_fn(64, 64, |x, y| {
            let v = (x.wrapping_mul(i + 3) ^ y.wrapping_mul(i * 7 + 1)) % 256;
            Luma([v as u8])
        });
        fx.row(
            &save(&fx.clip_dir, &format!("{i:04}.png"), &img),
            &fx.source,
            "",
            false,
        )
        .await;
    }
    let t = std::time::Instant::now();
    let s = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    println!(
        "dedup 1000 x 64x64: {:?} (scanned {}, marked {})",
        t.elapsed(),
        s.scanned,
        s.marked
    );

    let big = fixture().await;
    for i in 0..50u32 {
        let img = GrayImage::from_fn(1280, 720, |x, y| {
            Luma([((x / (i + 8)) ^ (y / (i + 5))).wrapping_mul(37) as u8])
        });
        big.row(
            &save(&big.clip_dir, &format!("{i:04}.png"), &img),
            &big.source,
            "",
            false,
        )
        .await;
    }
    let t = std::time::Instant::now();
    let s = dedup(&big.db, &big.dataset.id, None)
        .await
        .unwrap()
        .unwrap();
    println!(
        "dedup 50 x 1280x720: {:?} (scanned {}, marked {})",
        t.elapsed(),
        s.scanned,
        s.marked
    );
}
