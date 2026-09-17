//! Parsing `ai-toolkit`'s headless training output (spec appendix
//! "Progress"): the `tqdm` progress bar written to the run's log file, the
//! plain-text markers it prints around resume/OOM/completion, and the
//! on-disk layout it writes checkpoints and samples into.
//!
//! `ai-toolkit` redraws its `tqdm` bar in place with `\r`, so one chunk read
//! from the log can contain many stale bars followed by the current one;
//! [`split_updates`] and [`latest_step_from_updates`] exist so callers never
//! have to reason about that themselves. Markers ([`Marker`]) are the
//! handful of fixed, non-tqdm lines `ai-toolkit` prints for lifecycle events
//! that a progress bar can't express (resuming from a checkpoint, OOM
//! back-off, job completion or failure).
//!
//! No `regex` dependency: the line shapes are fixed enough (verified against
//! the pinned `ai-toolkit` commit) that hand-parsing with `split`/`find` is
//! simpler and avoids adding a crate for this alone.

use std::path::{Path, PathBuf};

use crate::CoreError;
use crate::Result;

/// One decoded `tqdm` progress update from the training log.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub step: u64,
    pub total: u64,
    pub loss: Option<f64>,
    pub lr: Option<f64>,
    pub eta_secs: Option<u64>,
}

/// A fixed, non-tqdm lifecycle line `ai-toolkit` prints to its log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
    /// `#### IMPORTANT RESUMING FROM <path> ####`
    Resuming(String),
    /// `Found step N in metadata, starting from there`
    FoundStep(u64),
    /// `# OOM during training step, skipping batch k/3 #`
    Oom { attempt: u32 },
    /// `OOM during training step 3 times in a row, aborting training`
    OomAbort,
    /// `Error running job: <msg>`
    Error(String),
    /// `Job stopped` (clean interrupt; no checkpoint is written)
    Stopped,
    /// The ` - 1 completed job` line that follows `Result:` on success.
    Completed,
    /// `Saved checkpoint to <path>`
    SavedCheckpoint(String),
}

/// The latest checkpoint and matching samples found under a run's work
/// directory (spec appendix "Output layout").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkDirState {
    pub latest_checkpoint: Option<(u64, PathBuf)>,
    pub latest_samples: Vec<PathBuf>,
}

/// Split a raw log chunk into individual `tqdm` updates: `ai-toolkit`
/// redraws the bar in place with `\r` and finishes a line with `\n`, so a
/// chunk read mid-stream contains both separators. Trims each update and
/// drops empty ones.
pub fn split_updates(buf: &str) -> impl Iterator<Item = &str> {
    buf.split(['\r', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Parse a single `tqdm` progress line. Returns `None` for any line that
/// isn't a progress bar.
///
/// The bar format is `<desc>: <pct>%|<fill>| <n>/<total> [<elapsed><<remaining>, <rate>, <postfix>]`.
/// `desc` (`job.name`) may itself contain spaces and colons, so this anchors
/// on the fixed `N/M [` shape right before the bracketed section rather than
/// trying to parse the description.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let open = line.find('[')?;
    let close = line.rfind(']')?;
    if close <= open {
        return None;
    }
    let before = line[..open].trim();
    let inside = &line[open + 1..close];

    let last_token = before.split_whitespace().last()?;
    let (step_str, total_str) = last_token.split_once('/')?;
    let step: u64 = step_str.parse().ok()?;
    let total: u64 = total_str.parse().ok()?;

    // "<elapsed><remaining>, <rate>, <postfix>" — the rate's own format
    // varies (`3.86it/s`, `3.86s/it`, `?it/s`) so it is skipped rather than
    // parsed; the postfix is optional (tqdm may not have set one yet).
    let mut segments = inside.splitn(3, ',');
    let time_part = segments.next().unwrap_or("");
    let _rate_part = segments.next();
    let postfix = segments.next().unwrap_or("").trim();

    let (_elapsed, remaining) = time_part.split_once('<').unwrap_or((time_part, ""));
    let eta_secs = parse_duration(remaining.trim());

    let lr = parse_labeled_f64(postfix, "lr:");
    let loss = parse_labeled_f64(postfix, "loss:");

    Some(Progress {
        step,
        total,
        loss,
        lr,
        eta_secs,
    })
}

/// Parse `mm:ss` or `h:mm:ss` into seconds; `?` (tqdm's placeholder for an
/// unknown remaining time) or anything else unparseable yields `None`.
fn parse_duration(s: &str) -> Option<u64> {
    if s.is_empty() || s.contains('?') {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [m, sec] => Some(m.trim().parse::<u64>().ok()? * 60 + sec.trim().parse::<u64>().ok()?),
        [h, m, sec] => Some(
            h.trim().parse::<u64>().ok()? * 3600
                + m.trim().parse::<u64>().ok()? * 60
                + sec.trim().parse::<u64>().ok()?,
        ),
        _ => None,
    }
}

/// Find `label` (e.g. `"lr:"`) in `postfix` and parse the whitespace-
/// delimited token right after it as an `f64`.
fn parse_labeled_f64(postfix: &str, label: &str) -> Option<f64> {
    let after = postfix.split_once(label)?.1.trim_start();
    let value: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
    value.parse().ok()
}

/// Find the exact text between `start` and the next `end` in `s`.
fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let after_start = s.split_once(start)?.1;
    let (middle, _) = after_start.split_once(end)?;
    Some(middle)
}

/// Parse a single log line for one of the fixed lifecycle [`Marker`]s.
/// Returns `None` for any other line (including `tqdm` bars).
pub fn parse_marker(line: &str) -> Option<Marker> {
    if let Some(path) = extract_between(line, "#### IMPORTANT RESUMING FROM ", " ####") {
        return Some(Marker::Resuming(path.to_string()));
    }
    if let Some(rest) = line
        .strip_prefix("Found step ")
        .and_then(|r| r.strip_suffix(" in metadata, starting from there"))
    {
        let step: u64 = rest.trim().parse().ok()?;
        return Some(Marker::FoundStep(step));
    }
    if let Some(attempt_str) =
        extract_between(line, "# OOM during training step, skipping batch ", "/3 #")
    {
        let attempt: u32 = attempt_str.trim().parse().ok()?;
        return Some(Marker::Oom { attempt });
    }
    if line.contains("OOM during training step 3 times in a row, aborting training") {
        return Some(Marker::OomAbort);
    }
    if let Some(msg) = line.strip_prefix("Error running job: ") {
        return Some(Marker::Error(msg.trim().to_string()));
    }
    if line.contains("Job stopped") {
        return Some(Marker::Stopped);
    }
    if line.contains(" - 1 completed job") {
        return Some(Marker::Completed);
    }
    if let Some(path) = line.strip_prefix("Saved checkpoint to ") {
        return Some(Marker::SavedCheckpoint(path.trim().to_string()));
    }
    None
}

/// Convenience over [`split_updates`] + [`parse_progress`]: the last
/// parseable progress update in a chunk, i.e. the run's current step.
pub fn latest_step_from_updates(buf: &str) -> Option<Progress> {
    let mut latest = None;
    for line in split_updates(buf) {
        if let Some(progress) = parse_progress(line) {
            latest = Some(progress);
        }
    }
    latest
}

/// Read the bytes appended to `path` since offset `from`, returning the
/// decoded (lossy UTF-8) chunk and the new offset. A missing file is not an
/// error: it just hasn't been created yet, so this returns an empty chunk
/// at the same offset.
pub async fn tail_log(path: &Path, from: u64) -> Result<(String, u64)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((String::new(), from)),
        Err(e) => return Err(CoreError::Io(e)),
    };

    let len = file.metadata().await.map_err(CoreError::Io)?.len();
    if from >= len {
        return Ok((String::new(), from));
    }

    file.seek(std::io::SeekFrom::Start(from))
        .await
        .map_err(CoreError::Io)?;
    let mut bytes = Vec::with_capacity((len - from) as usize);
    file.read_to_end(&mut bytes).await.map_err(CoreError::Io)?;

    let new_offset = from + bytes.len() as u64;
    let chunk = String::from_utf8_lossy(&bytes).into_owned();
    Ok((chunk, new_offset))
}

/// The `_{step:09}` suffix on a checkpoint file name
/// (`<run_name>_<step:09>.safetensors`), parsed against a specific run.
fn checkpoint_step(file_name: &str, run_name: &str) -> Option<u64> {
    let stem = file_name.strip_suffix(".safetensors")?;
    let step_str = stem.strip_prefix(run_name)?.strip_prefix('_')?;
    step_str.parse().ok()
}

/// The `_{step:09}_` step segment of a sample file name
/// (`<time>_<step:09>_<count>.<ext>`).
fn sample_step(file_name: &str) -> Option<u64> {
    let (_time, rest) = file_name.split_once('_')?;
    let (step_str, _count_and_ext) = rest.split_once('_')?;
    step_str.parse().ok()
}

/// Scan a run's work directory for its latest checkpoint and the samples
/// generated alongside it (spec appendix "Output layout"). A missing
/// directory (the run hasn't written anything yet) is not an error.
pub fn scan_work_dir(run_dir: &Path, run_name: &str) -> Result<WorkDirState> {
    if !run_dir.is_dir() {
        return Ok(WorkDirState::default());
    }

    let mut latest_checkpoint: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(run_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(step) = checkpoint_step(file_name, run_name) else {
            continue;
        };
        if latest_checkpoint.as_ref().is_none_or(|(s, _)| step > *s) {
            latest_checkpoint = Some((step, path));
        }
    }

    let samples_dir = run_dir.join("samples");
    let mut latest_samples = Vec::new();
    if samples_dir.is_dir() {
        let mut samples: Vec<(u64, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&samples_dir)? {
            let entry = entry?;
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(step) = sample_step(file_name) else {
                continue;
            };
            samples.push((step, path));
        }

        let target_step = match latest_checkpoint {
            Some((step, _)) => Some(step),
            None => samples.iter().map(|(step, _)| *step).max(),
        };
        if let Some(target_step) = target_step {
            latest_samples = samples
                .into_iter()
                .filter(|(step, _)| *step == target_step)
                .map(|(_, path)| path)
                .collect();
            latest_samples.sort();
        }
    }

    Ok(WorkDirState {
        latest_checkpoint,
        latest_samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_tqdm_progress_line() {
        let line = "my_lora:  12%|██        | 240/2000 [01:02<07:35,  3.86it/s, lr: 1.0e-04 loss: 3.123e-01]";
        let progress = parse_progress(line).expect("should parse a tqdm bar");
        assert_eq!(progress.step, 240);
        assert_eq!(progress.total, 2000);
        assert!((progress.loss.expect("loss") - 0.3123).abs() < 1e-9);
        assert!((progress.lr.expect("lr") - 1.0e-4).abs() < 1e-12);
        assert_eq!(progress.eta_secs, Some(455));
    }

    #[test]
    fn parses_slow_and_unknown_rates() {
        let slow =
            "job:   5%|▌         | 10/200 [00:38<12:03,  3.86s/it, lr: 1.0e-04 loss: 3.123e-01]";
        let p = parse_progress(slow).expect("should parse slow rate");
        assert_eq!(p.step, 10);
        assert_eq!(p.eta_secs, Some(12 * 60 + 3));

        let unknown = "job:   0%|          | 0/200 [00:00<?,  ?it/s]";
        let p2 = parse_progress(unknown).expect("should parse unknown rate/eta");
        assert_eq!(p2.step, 0);
        assert_eq!(p2.total, 200);
        assert_eq!(p2.eta_secs, None);
        assert_eq!(p2.loss, None);
        assert_eq!(p2.lr, None);

        let long_running =
            "job:  50%|█████     | 100/200 [1:02:03<2:00:00,  1.86it/s, lr: 1.0e-04 loss: 3.123e-01]";
        let p3 = parse_progress(long_running).expect("should parse h:mm:ss eta");
        assert_eq!(p3.eta_secs, Some(2 * 3600));
    }

    #[test]
    fn desc_with_colons_and_spaces_does_not_confuse_the_parser() {
        let line = "my lora: run 1:  12%|██        | 240/2000 [01:02<07:35,  3.86it/s, lr: 1.0e-04 loss: 3.123e-01]";
        let p = parse_progress(line).expect("should parse despite a busy desc");
        assert_eq!(p.step, 240);
        assert_eq!(p.total, 2000);
    }

    #[test]
    fn splits_carriage_return_updates_and_keeps_the_last() {
        let bar1 =
            "job:  10%|█         | 10/100 [00:01<00:09,  10.0it/s, lr: 1.0e-04 loss: 1.000e+00]";
        let bar2 =
            "job:  20%|██        | 20/100 [00:02<00:08,  10.0it/s, lr: 1.0e-04 loss: 9.000e-01]";
        let bar3 =
            "job:  30%|███       | 30/100 [00:03<00:07,  10.0it/s, lr: 1.0e-04 loss: 8.000e-01]";
        let buf = format!("{bar1}\r{bar2}\r{bar3}\n");

        let updates: Vec<&str> = split_updates(&buf).collect();
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0], bar1);
        assert_eq!(updates[2], bar3);

        let latest = latest_step_from_updates(&buf).expect("should find the last bar");
        assert_eq!(latest.step, 30);
    }

    #[test]
    fn detects_every_marker() {
        assert_eq!(
            parse_marker(
                "#### IMPORTANT RESUMING FROM /work/my_lora/my_lora_000000250.safetensors ####"
            ),
            Some(Marker::Resuming(
                "/work/my_lora/my_lora_000000250.safetensors".to_string()
            ))
        );
        assert_eq!(
            parse_marker("Found step 250 in metadata, starting from there"),
            Some(Marker::FoundStep(250))
        );
        assert_eq!(
            parse_marker("# OOM during training step, skipping batch 2/3 #"),
            Some(Marker::Oom { attempt: 2 })
        );
        assert_eq!(
            parse_marker("OOM during training step 3 times in a row, aborting training"),
            Some(Marker::OomAbort)
        );
        assert_eq!(
            parse_marker("Error running job: CUDA out of memory"),
            Some(Marker::Error("CUDA out of memory".to_string()))
        );
        assert_eq!(parse_marker("Job stopped"), Some(Marker::Stopped));
        assert_eq!(parse_marker(" - 1 completed job"), Some(Marker::Completed));
        assert_eq!(
            parse_marker("Saved checkpoint to /work/my_lora/my_lora_000000500.safetensors"),
            Some(Marker::SavedCheckpoint(
                "/work/my_lora/my_lora_000000500.safetensors".to_string()
            ))
        );
    }

    #[test]
    fn non_matching_lines_yield_none() {
        assert_eq!(parse_progress("just a log line"), None);
        assert_eq!(parse_progress("Result:"), None);
        assert_eq!(parse_marker("just a log line"), None);
        assert_eq!(parse_marker("Result:"), None);
        assert_eq!(
            parse_marker("my_lora:  12%|██ | 240/2000 [01:02<07:35, 3.86it/s]"),
            None
        );
    }

    #[tokio::test]
    async fn tail_log_reads_only_new_bytes_and_tolerates_a_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing.log");
        let (chunk, offset) = tail_log(&missing, 0).await.expect("missing file is ok");
        assert_eq!(chunk, "");
        assert_eq!(offset, 0);

        let log_path = dir.path().join("log.txt");
        tokio::fs::write(&log_path, b"first chunk\n")
            .await
            .expect("write");
        let (chunk1, offset1) = tail_log(&log_path, 0).await.expect("read from start");
        assert_eq!(chunk1, "first chunk\n");
        assert_eq!(offset1, 12);

        {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)
                .await
                .expect("open for append");
            file.write_all(b"second chunk\n").await.expect("append");
            file.flush().await.expect("flush");
        }

        let (chunk2, offset2) = tail_log(&log_path, offset1)
            .await
            .expect("read only new bytes");
        assert_eq!(chunk2, "second chunk\n");
        assert_eq!(offset2, offset1 + 13);
    }

    #[test]
    fn scans_checkpoints_and_samples() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path().join("my_lora");
        std::fs::create_dir_all(run_dir.join("samples")).expect("mkdir");
        std::fs::write(run_dir.join("my_lora_000000250.safetensors"), b"").expect("write");
        std::fs::write(run_dir.join("my_lora_000000500.safetensors"), b"").expect("write");
        let sample_250 = run_dir.join("samples").join("1700000000_000000250_0.png");
        let sample_500a = run_dir.join("samples").join("1700000100_000000500_0.png");
        let sample_500b = run_dir.join("samples").join("1700000100_000000500_1.png");
        std::fs::write(&sample_250, b"").expect("write");
        std::fs::write(&sample_500a, b"").expect("write");
        std::fs::write(&sample_500b, b"").expect("write");

        let state = scan_work_dir(&run_dir, "my_lora").expect("scan");
        assert_eq!(
            state.latest_checkpoint,
            Some((500, run_dir.join("my_lora_000000500.safetensors")))
        );
        assert_eq!(state.latest_samples, vec![sample_500a, sample_500b]);
    }

    #[test]
    fn scan_of_a_missing_dir_is_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does_not_exist");
        let state = scan_work_dir(&missing, "my_lora").expect("scan of missing dir");
        assert_eq!(state, WorkDirState::default());
    }
}
