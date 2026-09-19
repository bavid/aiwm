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
//!
//! Filter level C adds three more, cheap, non-ML checks on top of the above:
//! **dead frames** — a flat, (near-)black or (near-)white frame (a fade to
//! black, a blank slate) carries no visual information and only dilutes
//! training data; **transitions** — a frame that is *already blurry* is
//! further reclassified as a transition specifically, rather than plain
//! blur, when it is additionally very different (by perceptual hash) from
//! both its previous and next neighbour: that combination — blur *and* two
//! distant neighbours — is the signature of a cut smear or cross-fade, not a
//! real shot. Blur alone is still just blur, and a sharp frame is never a
//! transition, however different its neighbours are; **diversity cap** —
//! long, mostly-static clips can otherwise dump hundreds of near-identical
//! (but not quite duplicate) frames into a dataset, so a per-clip cap keeps
//! only the most visually spread-out subset via farthest-point selection on
//! the same perceptual hashes used for dedup.

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

/// Both numbers the dataset-wide dedup needs from one decode: the same
/// perceptual hash [`phash_of`] computes (same hasher config) and the
/// Laplacian variance [`is_blurry`] thresholds — higher means sharper.
pub fn phash_and_sharpness(path: &Path) -> Result<(ImageHash, f64)> {
    let img = image::open(path)
        .map_err(|e| dataset_err(format!("read {} for dedup check: {e}", path.display())))?;
    let hasher = HasherConfig::new().to_hasher();
    Ok((hasher.hash_image(&img), laplacian_variance(&img.to_luma8())))
}

/// Why a frame was dropped. Stored on the row (`dataset_frames.rejection_reason`)
/// so the curation grid can show each reason as a filter chip and restore
/// individual frames. `""` on the row means "kept".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    Black,
    Transition,
    Blur,
    Duplicate,
    Cap,
    /// Clip mode only: undecodable or shorter than the minimum.
    Unusable,
    /// Marked by the dataset-wide dedup (`housekeeping::dedup`): a near-copy
    /// of a sharper frame anywhere in the dataset, not just its neighbour.
    DuplicateGlobal,
}

impl RejectionReason {
    pub const ALL: [RejectionReason; 7] = [
        Self::Black,
        Self::Transition,
        Self::Blur,
        Self::Duplicate,
        Self::Cap,
        Self::Unusable,
        Self::DuplicateGlobal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Transition => "transition",
            Self::Blur => "blur",
            Self::Duplicate => "duplicate",
            Self::Cap => "cap",
            Self::Unusable => "unusable",
            Self::DuplicateGlobal => "duplicate_global",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

/// A frame whose mean luma sits within this many levels of pure black (0) or
/// pure white (255) *and* whose pixels barely vary is a fade/blank frame.
const DEAD_FRAME_LUMA_MARGIN: f64 = 12.0;
const DEAD_FRAME_MAX_STDDEV: f64 = 6.0;

/// Hamming distance above which two frames count as "different pictures"
/// for transition detection — deliberately far above the duplicate
/// threshold (6): a cut between two scenes is *very* different, a slow pan
/// is not.
pub const DEFAULT_TRANSITION_MIN_DISTANCE: u32 = 20;

/// Per-clip diversity cap default (spec 3, filter 4). `0` = unlimited.
pub const DEFAULT_MAX_FRAMES_PER_CLIP: usize = 40;

fn luma_mean_and_stddev(gray: &image::GrayImage) -> (f64, f64) {
    let n = (gray.width() as f64) * (gray.height() as f64);
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = gray.pixels().map(|p| f64::from(p.0[0])).sum::<f64>() / n;
    let var = gray
        .pixels()
        .map(|p| {
            let d = f64::from(p.0[0]) - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    (mean, var.sqrt())
}

/// (Nearly) all-black or all-white with almost no variation — a fade, a
/// blank, a dead frame between scenes.
pub fn is_dead_frame(path: &Path) -> Result<bool> {
    let img = image::open(path)
        .map_err(|e| dataset_err(format!("read {} for dead-frame check: {e}", path.display())))?;
    let (mean, stddev) = luma_mean_and_stddev(&img.to_luma8());
    let near_black = mean <= DEAD_FRAME_LUMA_MARGIN;
    let near_white = mean >= 255.0 - DEAD_FRAME_LUMA_MARGIN;
    Ok((near_black || near_white) && stddev <= DEAD_FRAME_MAX_STDDEV)
}

/// A transition (cut smear / cross-fade) is a *blurry* frame that is very
/// different from both its previous and its next neighbour. Either
/// neighbour missing (first/last frame) means "not a transition".
pub fn is_transition(
    blurry: bool,
    prev: Option<&ImageHash>,
    this: &ImageHash,
    next: Option<&ImageHash>,
    min_distance: u32,
) -> bool {
    let (Some(prev), Some(next)) = (prev, next) else {
        return false;
    };
    blurry && prev.dist(this) > min_distance && next.dist(this) > min_distance
}

/// Farthest-point selection: keep at most `cap` of `hashes` (0 = all),
/// always starting with index 0, then repeatedly the frame whose *minimum*
/// distance to everything already chosen is largest. Returns indices in
/// their original order.
pub fn select_diverse(hashes: &[ImageHash], cap: usize) -> Vec<usize> {
    if cap == 0 || hashes.len() <= cap {
        return (0..hashes.len()).collect();
    }
    let mut chosen: Vec<usize> = vec![0];
    let mut is_chosen = vec![false; hashes.len()];
    is_chosen[0] = true;
    let mut min_dist: Vec<u32> = hashes.iter().map(|h| hashes[0].dist(h)).collect();
    while chosen.len() < cap {
        let Some((best, _)) = min_dist
            .iter()
            .enumerate()
            .filter(|(i, _)| !is_chosen[*i])
            // Ties favour the lower index: `Reverse(i)` grows as `i` shrinks,
            // so `max_by_key` picks the earliest tied frame deterministically.
            .max_by_key(|(i, d)| (**d, std::cmp::Reverse(*i)))
        else {
            break;
        };
        chosen.push(best);
        is_chosen[best] = true;
        for (i, d) in min_dist.iter_mut().enumerate() {
            *d = (*d).min(hashes[best].dist(&hashes[i]));
        }
    }
    chosen.sort_unstable();
    chosen
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
    fn phash_of_separates_identical_pictures_from_genuinely_different_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b_same = tmp.path().join("b.png");
        let c_diff = tmp.path().join("c.png");
        write_png(&a, &sharp_checkerboard(32));
        write_png(&b_same, &sharp_checkerboard(32)); // identical picture
        write_png(&c_diff, &flat_gray(32, 10)); // a completely different image

        let hash_a = phash_of(&a).unwrap();
        assert!(
            hash_a.dist(&phash_of(&b_same).unwrap()) <= DEFAULT_PHASH_MAX_DISTANCE,
            "an identical frame reads as a duplicate"
        );
        assert!(
            hash_a.dist(&phash_of(&c_diff).unwrap()) > DEFAULT_PHASH_MAX_DISTANCE,
            "a genuinely different frame does not"
        );
    }

    #[test]
    fn phash_of_reports_a_clear_error_for_a_missing_file() {
        let err = phash_of(Path::new("C:\\nope\\gone.png")).unwrap_err();
        assert!(err.to_string().contains("dedup check"), "{err}");
    }

    #[test]
    fn rejection_reason_round_trips_through_its_string_form() {
        for r in RejectionReason::ALL {
            assert_eq!(RejectionReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(RejectionReason::parse(""), None);
        assert_eq!(RejectionReason::parse("nonsense"), None);
    }

    #[test]
    fn duplicate_global_is_a_known_reason() {
        assert_eq!(
            RejectionReason::parse("duplicate_global"),
            Some(RejectionReason::DuplicateGlobal)
        );
        assert_eq!(
            RejectionReason::DuplicateGlobal.as_str(),
            "duplicate_global"
        );
    }

    #[test]
    fn phash_and_sharpness_matches_the_separate_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let sharp = tmp.path().join("sharp.png");
        let flat = tmp.path().join("flat.png");
        write_png(&sharp, &sharp_checkerboard(32));
        write_png(&flat, &flat_gray(32, 128));

        let (hash, sharpness) = phash_and_sharpness(&sharp).unwrap();
        assert_eq!(hash, phash_of(&sharp).unwrap());
        assert!(sharpness > DEFAULT_BLUR_THRESHOLD);
        let (_, flat_sharpness) = phash_and_sharpness(&flat).unwrap();
        assert_eq!(flat_sharpness, 0.0);
        assert!(phash_and_sharpness(&tmp.path().join("gone.png")).is_err());
    }

    #[test]
    fn is_dead_frame_flags_black_and_white_but_not_mid_gray_or_content() {
        let tmp = tempfile::tempdir().unwrap();
        let black = tmp.path().join("black.png");
        let white = tmp.path().join("white.png");
        let gray = tmp.path().join("gray.png");
        let content = tmp.path().join("content.png");
        write_png(&black, &flat_gray(32, 3));
        write_png(&white, &flat_gray(32, 252));
        write_png(&gray, &flat_gray(32, 128));
        write_png(&content, &sharp_checkerboard(32));

        assert!(is_dead_frame(&black).unwrap());
        assert!(is_dead_frame(&white).unwrap());
        assert!(
            !is_dead_frame(&gray).unwrap(),
            "flat mid-gray is blurry, not dead"
        );
        assert!(!is_dead_frame(&content).unwrap());

        // Pin luma_mean_and_stddev itself for these fixtures: flat images have
        // zero variance, and a 50/50 checkerboard has an exactly-computable
        // mean/stddev — both catch drift in the helper, not just the thresholds.
        assert_eq!(
            luma_mean_and_stddev(&flat_gray(32, 3).to_luma8()),
            (3.0, 0.0)
        );
        assert_eq!(
            luma_mean_and_stddev(&flat_gray(32, 252).to_luma8()),
            (252.0, 0.0)
        );
        assert_eq!(
            luma_mean_and_stddev(&sharp_checkerboard(32).to_luma8()),
            (127.5, 127.5)
        );
    }

    #[test]
    fn is_dead_frame_pins_the_luma_margin_thresholds() {
        let tmp = tempfile::tempdir().unwrap();
        let margin = DEAD_FRAME_LUMA_MARGIN as u8;

        let black_at_margin = tmp.path().join("black_at_margin.png");
        let black_past_margin = tmp.path().join("black_past_margin.png");
        write_png(&black_at_margin, &flat_gray(32, margin));
        write_png(&black_past_margin, &flat_gray(32, margin + 1));
        assert!(
            is_dead_frame(&black_at_margin).unwrap(),
            "mean == margin is still dead (near_black uses <=)"
        );
        assert!(
            !is_dead_frame(&black_past_margin).unwrap(),
            "mean == margin + 1 is no longer dead"
        );

        let white_at_margin = tmp.path().join("white_at_margin.png");
        let white_before_margin = tmp.path().join("white_before_margin.png");
        write_png(&white_at_margin, &flat_gray(32, 255 - margin));
        write_png(&white_before_margin, &flat_gray(32, 254 - margin));
        assert!(
            is_dead_frame(&white_at_margin).unwrap(),
            "mean == 255 - margin is still dead (near_white uses >=)"
        );
        assert!(
            !is_dead_frame(&white_before_margin).unwrap(),
            "mean == 254 - margin is not dead"
        );
    }

    #[test]
    fn is_transition_needs_blur_and_two_distant_neighbors() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        write_png(&a, &sharp_checkerboard(32));
        write_png(&b, &flat_gray(32, 10));
        let ha = phash_of(&a).unwrap();
        let hb = phash_of(&b).unwrap();
        let far = ha.dist(&hb);
        assert!(
            far > DEFAULT_TRANSITION_MIN_DISTANCE,
            "fixture must be distant: {far}"
        );

        // Blurry and far from both neighbours -> transition.
        assert!(is_transition(
            true,
            Some(&ha),
            &hb,
            Some(&ha),
            DEFAULT_TRANSITION_MIN_DISTANCE
        ));
        // Sharp -> never a transition, however different the neighbours are.
        assert!(!is_transition(
            false,
            Some(&ha),
            &hb,
            Some(&ha),
            DEFAULT_TRANSITION_MIN_DISTANCE
        ));
        // Blurry but similar to one neighbour -> not a transition.
        assert!(!is_transition(
            true,
            Some(&hb),
            &hb,
            Some(&ha),
            DEFAULT_TRANSITION_MIN_DISTANCE
        ));
        // First/last frame (a missing neighbour) is never a transition.
        assert!(!is_transition(
            true,
            None,
            &hb,
            Some(&ha),
            DEFAULT_TRANSITION_MIN_DISTANCE
        ));
    }

    #[test]
    fn is_transition_pins_the_min_distance_threshold_as_a_strict_greater_than() {
        // Built directly via the crate's own public `ImageHash::from_bytes`
        // (no need for a test-only constructor) so the Hamming distance is
        // exact rather than incidental to a PNG fixture. `zero` is all-0-bits;
        // `at_boundary` sets exactly DEFAULT_TRANSITION_MIN_DISTANCE bits, and
        // `over_boundary` one more.
        let zero =
            ImageHash::from_bytes(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();
        // 16 bits (0xFF, 0xFF) + 4 bits (0x0F) = 20 = DEFAULT_TRANSITION_MIN_DISTANCE.
        let at_boundary =
            ImageHash::from_bytes(&[0xFF, 0xFF, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();
        // 16 bits (0xFF, 0xFF) + 5 bits (0x1F) = 21 = DEFAULT_TRANSITION_MIN_DISTANCE + 1.
        let over_boundary =
            ImageHash::from_bytes(&[0xFF, 0xFF, 0x1F, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();

        assert_eq!(zero.dist(&at_boundary), DEFAULT_TRANSITION_MIN_DISTANCE);
        assert_eq!(
            zero.dist(&over_boundary),
            DEFAULT_TRANSITION_MIN_DISTANCE + 1
        );

        // Distance == threshold: the check is `>`, so this is NOT a transition.
        assert!(!is_transition(
            true,
            Some(&zero),
            &at_boundary,
            Some(&zero),
            DEFAULT_TRANSITION_MIN_DISTANCE
        ));
        // Distance == threshold + 1: now it IS a transition.
        assert!(is_transition(
            true,
            Some(&zero),
            &over_boundary,
            Some(&zero),
            DEFAULT_TRANSITION_MIN_DISTANCE
        ));
    }

    #[test]
    fn select_diverse_keeps_everything_under_the_cap_and_prefers_spread_above_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut hashes = Vec::new();
        // 3 near-identical checkerboards, 1 flat dark, 1 flat light.
        for i in 0..3 {
            let p = tmp.path().join(format!("c{i}.png"));
            write_png(&p, &sharp_checkerboard(32));
            hashes.push(phash_of(&p).unwrap());
        }
        let dark = tmp.path().join("dark.png");
        let light = tmp.path().join("light.png");
        write_png(&dark, &flat_gray(32, 10));
        write_png(&light, &flat_gray(32, 240));
        hashes.push(phash_of(&dark).unwrap());
        hashes.push(phash_of(&light).unwrap());

        assert_eq!(
            select_diverse(&hashes, 0),
            (0..5).collect::<Vec<_>>(),
            "0 = unlimited"
        );
        assert_eq!(select_diverse(&hashes, 10), (0..5).collect::<Vec<_>>());

        let picked = select_diverse(&hashes, 2);
        assert_eq!(picked.len(), 2);
        assert!(picked.contains(&0), "always starts with the first frame");
        assert!(
            picked.contains(&3) || picked.contains(&4),
            "second pick is the most different"
        );
        let mut sorted = picked.clone();
        sorted.sort_unstable();
        assert_eq!(picked, sorted, "returned in original order");
    }

    #[test]
    fn select_diverse_breaks_an_exact_tie_toward_the_lower_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mut hashes = Vec::new();
        for i in 0..3 {
            let p = tmp.path().join(format!("identical{i}.png"));
            write_png(&p, &sharp_checkerboard(32));
            hashes.push(phash_of(&p).unwrap());
        }
        // All three are the same picture, so every pairwise distance is 0 —
        // a genuine tie with no "most different" frame to break it.
        assert_eq!(hashes[0].dist(&hashes[1]), 0);
        assert_eq!(hashes[0].dist(&hashes[2]), 0);

        assert_eq!(select_diverse(&hashes, 2), vec![0, 1]);
    }
}
