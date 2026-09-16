//! Stage 3: quality filtering. Two explicitly named problems (docs/TODO.md):
//! blur and near-duplicate frames. Both are real, standard, non-ML
//! techniques — no model, no GPU, just pixels.
//!
//! **Blur** — variance of the Laplacian: convolve the grayscale image with
//! the discrete Laplacian kernel, then take the variance of the result. A
//! sharp image has a lot of high-frequency edge content, so the Laplacian
//! response varies a lot pixel-to-pixel (high variance); a blurry image's
//! edges are smeared out, so the response is nearly flat everywhere (low
//! variance). This is hand-rolled rather than routed through `imageproc`'s
//! generic `filter3x3` deliberately: that function clamps its output to the
//! target pixel type (e.g. `u8`), which would clip every negative Laplacian
//! response to 0 and silently bias the variance downward — exactly the
//! wrong thing for a threshold meant to separate sharp from blurry. Keeping
//! the convolution's signed `i32` result until the variance step (the same
//! thing `cv2.Laplacian(..., ddepth=CV_64F)` guards against in OpenCV, the
//! reference implementation of this exact technique) keeps the numbers
//! comparable to the published rule-of-thumb thresholds this module's
//! default is drawn from.
//!
//! **Near-duplicates** — perceptual hashing via the battle-tested
//! `image_hasher` crate (the maintained continuation of `img_hash`):
//! adjacent kept frames within the same source video are hashed and compared
//! by Hamming distance; a small distance means "practically the same
//! picture" (a static shot, a slow pan) and only the first of the run is
//! kept. Comparing each candidate only against the *last kept* frame in its
//! own group (not every other frame — O(n) instead of O(n^2)) is deliberate:
//! within one source video, near-duplicates are runs of consecutive frames
//! by construction (temporal coherence), not scattered across the timeline.

use std::path::Path;

use image_hasher::{HasherConfig, ImageHash};

use crate::Result;

use super::dataset_err;

/// Below this, a frame is dropped as too blurry to be useful training data.
/// A commonly cited starting point for this exact technique (variance of
/// Laplacian on an 8-bit grayscale image) — real photos/renders comfortably
/// clear this; a genuinely soft-focus or motion-blurred frame does not.
pub const DEFAULT_BLUR_THRESHOLD: f64 = 100.0;
pub const MIN_BLUR_THRESHOLD: f64 = 0.0;
pub const MAX_BLUR_THRESHOLD: f64 = 10_000.0;

/// Hamming distance (out of a 64-bit perceptual hash) at or below which two
/// frames count as "the same picture" and the later one is dropped.
pub const DEFAULT_PHASH_MAX_DISTANCE: u32 = 6;
pub const MAX_PHASH_DISTANCE: u32 = 64;

const LAPLACIAN_KERNEL: [i32; 9] = [0, 1, 0, 1, -4, 1, 0, 1, 0];

/// Variance of the 3x3 discrete Laplacian response over a grayscale image.
/// Edge pixels use replicated-border sampling (clamped coordinates) so every
/// pixel — including the border — contributes, rather than shrinking the
/// output or padding with zeros (which would artificially suppress the
/// variance right at the border).
fn laplacian_variance(gray: &image::GrayImage) -> f64 {
    let (w, h) = gray.dimensions();
    if w == 0 || h == 0 {
        return 0.0;
    }
    let at = |x: i64, y: i64| -> i32 {
        let cx = x.clamp(0, i64::from(w) - 1) as u32;
        let cy = y.clamp(0, i64::from(h) - 1) as u32;
        i32::from(gray.get_pixel(cx, cy).0[0])
    };

    let mut responses: Vec<i32> = Vec::with_capacity((w * h) as usize);
    for y in 0..i64::from(h) {
        for x in 0..i64::from(w) {
            let mut acc = 0i32;
            let mut k = 0usize;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    acc += LAPLACIAN_KERNEL[k] * at(x + dx, y + dy);
                    k += 1;
                }
            }
            responses.push(acc);
        }
    }

    let n = responses.len() as f64;
    let mean = responses.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
    responses
        .iter()
        .map(|&v| {
            let d = f64::from(v) - mean;
            d * d
        })
        .sum::<f64>()
        / n
}

/// Load `path`, decide whether it is too blurry to keep.
pub fn is_blurry(path: &Path, threshold: f64) -> Result<bool> {
    let img = image::open(path)
        .map_err(|e| dataset_err(format!("read {} for blur check: {e}", path.display())))?;
    let gray = img.to_luma8();
    Ok(laplacian_variance(&gray) < threshold)
}

/// A perceptual hash for near-duplicate detection.
pub fn phash_of(path: &Path) -> Result<ImageHash> {
    let img = image::open(path)
        .map_err(|e| dataset_err(format!("read {} for dedup check: {e}", path.display())))?;
    let hasher = HasherConfig::new().to_hasher();
    Ok(hasher.hash_image(&img))
}

/// Drop `path` if it is a near-duplicate of `last_kept` (when there is a
/// previous kept frame in the same group). Returns the hash to remember as
/// "last kept" when `path` survives, or `None` when it was dropped.
pub fn dedup_step(
    path: &Path,
    last_kept: Option<&ImageHash>,
    max_distance: u32,
) -> Result<Option<ImageHash>> {
    let hash = phash_of(path)?;
    if let Some(prev) = last_kept {
        if prev.dist(&hash) <= max_distance {
            return Ok(None);
        }
    }
    Ok(Some(hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    fn write_png(path: &Path, img: &image::DynamicImage) {
        img.save(path).unwrap();
    }

    fn sharp_checkerboard(size: u32) -> image::DynamicImage {
        let mut buf = GrayImage::new(size, size);
        for y in 0..size {
            for x in 0..size {
                let v = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 };
                buf.put_pixel(x, y, Luma([v]));
            }
        }
        image::DynamicImage::ImageLuma8(buf)
    }

    fn flat_gray(size: u32, value: u8) -> image::DynamicImage {
        image::DynamicImage::ImageLuma8(GrayImage::from_pixel(size, size, Luma([value])))
    }

    #[test]
    fn laplacian_variance_is_zero_for_a_perfectly_flat_image() {
        let flat = GrayImage::from_pixel(16, 16, Luma([128]));
        assert_eq!(laplacian_variance(&flat), 0.0);
    }

    #[test]
    fn laplacian_variance_is_high_for_a_sharp_checkerboard() {
        let flat = GrayImage::from_pixel(32, 32, Luma([128]));
        let sharp = match sharp_checkerboard(32) {
            image::DynamicImage::ImageLuma8(g) => g,
            _ => unreachable!(),
        };
        assert!(laplacian_variance(&sharp) > laplacian_variance(&flat));
        assert!(laplacian_variance(&sharp) > DEFAULT_BLUR_THRESHOLD);
    }

    #[test]
    fn is_blurry_flags_a_flat_image_and_spares_a_sharp_one() {
        let tmp = tempfile::tempdir().unwrap();
        let flat_path = tmp.path().join("flat.png");
        let sharp_path = tmp.path().join("sharp.png");
        write_png(&flat_path, &flat_gray(32, 200));
        write_png(&sharp_path, &sharp_checkerboard(32));

        assert!(is_blurry(&flat_path, DEFAULT_BLUR_THRESHOLD).unwrap());
        assert!(!is_blurry(&sharp_path, DEFAULT_BLUR_THRESHOLD).unwrap());
    }

    #[test]
    fn is_blurry_reports_a_clear_error_for_an_unreadable_file() {
        let tmp = tempfile::tempdir().unwrap();
        let bogus = tmp.path().join("not-an-image.png");
        std::fs::write(&bogus, b"not a png").unwrap();
        let err = is_blurry(&bogus, DEFAULT_BLUR_THRESHOLD).unwrap_err();
        assert!(err.to_string().contains("blur check"), "{err}");
    }

    #[test]
    fn dedup_step_keeps_the_first_frame_when_there_is_no_prior_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.png");
        write_png(&path, &sharp_checkerboard(32));
        let kept = dedup_step(&path, None, DEFAULT_PHASH_MAX_DISTANCE).unwrap();
        assert!(kept.is_some());
    }

    #[test]
    fn dedup_step_drops_a_near_identical_successor_and_keeps_a_distinct_one() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b_same = tmp.path().join("b.png");
        let c_diff = tmp.path().join("c.png");
        write_png(&a, &sharp_checkerboard(32));
        write_png(&b_same, &sharp_checkerboard(32)); // identical picture
        write_png(&c_diff, &flat_gray(32, 10)); // a completely different image

        let hash_a = dedup_step(&a, None, DEFAULT_PHASH_MAX_DISTANCE)
            .unwrap()
            .unwrap();
        let after_b = dedup_step(&b_same, Some(&hash_a), DEFAULT_PHASH_MAX_DISTANCE).unwrap();
        assert!(after_b.is_none(), "an identical frame must be dropped");

        let after_c = dedup_step(&c_diff, Some(&hash_a), DEFAULT_PHASH_MAX_DISTANCE).unwrap();
        assert!(
            after_c.is_some(),
            "a genuinely different frame must survive"
        );
    }

    #[test]
    fn phash_of_reports_a_clear_error_for_a_missing_file() {
        let err = phash_of(Path::new("C:\\nope\\gone.png")).unwrap_err();
        assert!(err.to_string().contains("dedup check"), "{err}");
    }
}
