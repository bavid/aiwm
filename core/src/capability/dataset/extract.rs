//! Stage 2: frame extraction. Single images make better training data than
//! raw video (the user's own observation, docs/TODO.md) — so every video is
//! decoded down to a handful of still frames per second via `ffmpeg`, rather
//! than kept as video. Sampling at ~1-2 fps (not every frame) skips the
//! near-duplicate noise between consecutive frames of the same shot; the
//! duplicate filter (`super::filter`) still catches whatever slips through a
//! slow pan or a static shot.
//!
//! `ffmpeg` is resolved the same way [`crate::sidecar::resolve_uv`] resolves
//! `uv`: PATH first, then common install locations — never bundled, since
//! most workstations doing local AI work already have it (ComfyUI's own
//! `SaveVideo` node needs it too — docs/TODO.md's 4.0 entry).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::Result;

use super::dataset_err;

/// Frame-extraction plans below this many frames/second under-sample a clip
/// (many real seconds per still); above the ceiling it's effectively "every
/// frame", which is exactly the near-duplicate noise this stage exists to
/// avoid.
pub const MIN_SAMPLE_FPS: f64 = 0.1;
pub const MAX_SAMPLE_FPS: f64 = 10.0;
pub const DEFAULT_SAMPLE_FPS: f64 = 1.5;

/// Locate the `ffmpeg` executable: PATH, then common Windows install spots
/// (winget's shim, and the common `Program Files` layout `gyan.dev` builds
/// use). Mirrors `sidecar::resolve_uv`'s shape exactly.
pub fn resolve_ffmpeg() -> Option<PathBuf> {
    let names = ["ffmpeg.exe", "ffmpeg"];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in names {
                let cand = dir.join(name);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        // WinGet shims a `Links` folder onto PATH for most installs, but a
        // machine where that shim is missing (a stale PATH cache) still has
        // the real binary here.
        let cand = local
            .join("Microsoft")
            .join("WinGet")
            .join("Links")
            .join("ffmpeg.exe");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// The ffmpeg CLI invocation for sampling `video` at `fps` frames/second into
/// `out_dir/frame_%06d.png`. A pure builder (no process spawned) so the exact
/// command is unit-testable without ffmpeg installed.
pub fn ffmpeg_extract_args(video: &Path, out_dir: &Path, fps: f64) -> Vec<String> {
    vec![
        "-y".into(),
        "-i".into(),
        video.to_string_lossy().into_owned(),
        "-vf".into(),
        format!("fps={fps}"),
        "-vsync".into(),
        "0".into(),
        out_dir
            .join("frame_%06d.png")
            .to_string_lossy()
            .into_owned(),
    ]
}

/// One extracted still, with its approximate position in the source video.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedFrame {
    pub path: PathBuf,
    pub timestamp_secs: f64,
}

/// `ffmpeg`'s own `frame_%06d.png` numbering is 1-based; frame `n` sits at
/// roughly `(n - 1) / fps` seconds (the fps filter samples at a fixed
/// interval starting near t=0). Approximate, not frame-accurate — good
/// enough to pick "a bit later" neighbors for temporal-context captioning.
pub fn timestamp_for_index(index: u32, fps: f64) -> f64 {
    if fps <= 0.0 {
        return 0.0;
    }
    f64::from(index.saturating_sub(1)) / fps
}

/// Run `ffmpeg` against `video`, sampling at `fps`, writing numbered PNGs
/// into `out_dir` (created if missing). Returns the extracted frames in
/// order with their approximate timestamps.
pub async fn extract_frames(
    ffmpeg_bin: &Path,
    video: &Path,
    out_dir: &Path,
    fps: f64,
) -> Result<Vec<ExtractedFrame>> {
    tokio::fs::create_dir_all(out_dir)
        .await
        .map_err(|e| dataset_err(format!("create {}: {e}", out_dir.display())))?;

    let args = ffmpeg_extract_args(video, out_dir, fps);
    let output = Command::new(ffmpeg_bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| dataset_err(format!("spawn ffmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(dataset_err(format!(
            "ffmpeg failed on {}: {}",
            video.display(),
            first_lines(&stderr, 4)
        )));
    }

    let mut names: Vec<PathBuf> = std::fs::read_dir(out_dir)
        .map_err(|e| dataset_err(format!("read {}: {e}", out_dir.display())))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    names.sort();

    Ok(names
        .into_iter()
        .enumerate()
        .map(|(i, path)| ExtractedFrame {
            path,
            timestamp_secs: timestamp_for_index(u32::try_from(i + 1).unwrap_or(u32::MAX), fps),
        })
        .collect())
}

fn first_lines(s: &str, n: usize) -> String {
    s.lines().take(n).collect::<Vec<_>>().join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_args_build_a_sane_ffmpeg_invocation() {
        let args = ffmpeg_extract_args(
            Path::new("E:\\Data\\Ghibli\\clip.mp4"),
            Path::new("C:\\out\\job-1"),
            1.5,
        );
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "-i");
        assert_eq!(args[2], "E:\\Data\\Ghibli\\clip.mp4");
        assert!(args.contains(&"fps=1.5".to_string()));
        assert!(args.last().unwrap().ends_with("frame_%06d.png"));
    }

    #[test]
    fn timestamp_for_index_starts_at_zero_and_scales_with_fps() {
        assert_eq!(timestamp_for_index(1, 2.0), 0.0);
        assert_eq!(timestamp_for_index(3, 2.0), 1.0);
        assert_eq!(timestamp_for_index(0, 2.0), 0.0);
        assert_eq!(timestamp_for_index(5, 0.0), 0.0);
    }

    // A real `ffmpeg` spawn is an integration concern (needs the binary and a
    // real container on disk); `resolve_ffmpeg` finding a real PATH entry is
    // exercised implicitly wherever `ffmpeg -version` succeeded on this
    // machine during development, not asserted here (CI may have no ffmpeg
    // at all -- same reasoning as the sidecar's `sidecar_or_skip!` tests).
    #[tokio::test]
    async fn extract_frames_reports_a_clear_error_for_a_missing_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let err = extract_frames(
            Path::new("C:\\definitely\\not\\ffmpeg.exe"),
            Path::new("E:\\Data\\x\\clip.mp4"),
            tmp.path(),
            1.5,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("spawn ffmpeg"), "{err}");
    }
}
