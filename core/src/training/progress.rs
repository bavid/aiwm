//! Parsing `ai-toolkit`'s headless training output (spec appendix
//! "Progress"): the `tqdm` progress bar written to the run's log file, the
//! plain-text markers it prints around resume/OOM/completion, and the
//! on-disk layout it writes checkpoints and samples into.
//!
//! `ai-toolkit` redraws its `tqdm` bar in place with `\r`, so one chunk read
//! from the log can contain many stale bars — including a final, still-being
//! -written fragment with no closing `]` — followed by the current complete
//! one; [`split_updates`] and [`latest_step_from_updates`] exist so callers
//! never have to reason about that themselves. Markers ([`Marker`]) are the
//! handful of fixed, non-tqdm lines `ai-toolkit` prints for lifecycle events
//! that a progress bar can't express (resuming from a checkpoint, OOM
//! back-off, job completion or failure). [`tail_log`] also copes with the
//! log file being truncated or recreated shorter than the reader's last
//! offset, treating that as "read from the start again" rather than
//! "nothing new".
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
    /// The ` - 1 completed job` line that follows `Result:` on success
    /// (matched with or without its leading indentation).
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
/// `desc` (`job.name`) may itself contain spaces, colons or even brackets
/// (e.g. `my [job]: 12%|...`), so this anchors on the *last* `[...]` pair in
/// the line — the closing `]` via `rfind`, then the matching `[` via
/// `rfind` on everything before it — rather than the first `[`, and takes
/// the `N/M` step/total from the last whitespace-separated token before
/// that bracket rather than trying to parse the description.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let close = line.rfind(']')?;
    let open = line[..close].rfind('[')?;
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

/// Find `label` (e.g. `"lr:"`) in `postfix` at a token boundary — preceded
/// by whitespace, a comma, or the start of the string — and parse the
/// whitespace-delimited token right after it as an `f64`. The boundary
/// check keeps a look-alike key from shadowing the real one, e.g.
/// `avg_loss: 9.999 loss: 3.123e-01` must still yield `loss: 3.123e-01`,
/// not the digits inside `avg_loss:`.
fn parse_labeled_f64(postfix: &str, label: &str) -> Option<f64> {
    let mut search_from = 0;
    while let Some(rel_idx) = postfix[search_from..].find(label) {
        let idx = search_from + rel_idx;
        let at_boundary = idx == 0
            || postfix[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || c == ',');
        if at_boundary {
            let after = postfix[idx + label.len()..].trim_start();
            let value: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
            if let Ok(parsed) = value.parse() {
                return Some(parsed);
            }
        }
        search_from = idx + label.len();
    }
    None
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
    // Deliberately matched without the leading space of the real line
    // (` - 1 completed job`): every caller goes through [`split_updates`],
    // which trims each update, so requiring the indentation here would make
    // completion undetectable for exactly the code that needs it.
    if line.contains("- 1 completed job") {
        return Some(Marker::Completed);
    }
    if let Some(path) = line.strip_prefix("Saved checkpoint to ") {
        return Some(Marker::SavedCheckpoint(path.trim().to_string()));
    }
    None
}

/// Parse a `tqdm` line **only if it is this run's own training bar**.
///
/// `ai-toolkit` draws several other bars with exactly the same shape before
/// and during a run — quantising the transformer's blocks, caching latents,
/// generating sample images — and [`parse_progress`] cannot tell them apart,
/// because it deliberately does not look at the description. Feeding those to
/// the runner makes a run report a step it is nowhere near and, worse,
/// rewrite its own `total_steps` to the other bar's total: the first real run
/// showed `step 34` (out of the latent cache's 50) while the log still said
/// "Generating baseline samples before training".
///
/// The training bar is the one `tqdm` gets `desc=job.name` for, so it is the
/// one that starts `"<run name>: "`. Matching on the name rather than on a
/// list of the other bars' descriptions keeps this correct when a future
/// `ai-toolkit` adds a fourth setup bar.
pub fn parse_run_progress(line: &str, run_name: &str) -> Option<Progress> {
    let rest = line.strip_prefix(run_name)?;
    // `desc` is followed by tqdm's `": "`. Requiring the colon is what stops
    // a run called `myrender` from claiming `myrender-v1`'s bar.
    if !rest.starts_with(':') {
        return None;
    }
    parse_progress(line)
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
///
/// If `from` is *past* the file's current length — the log was truncated or
/// recreated shorter than where the reader last left off — this resets to
/// offset 0 and reads from the start instead of treating it as "nothing
/// new". Only `from == len` genuinely means there is nothing new yet.
pub async fn tail_log(path: &Path, from: u64) -> Result<(String, u64)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((String::new(), from)),
        Err(e) => return Err(CoreError::Io(e)),
    };

    let len = file.metadata().await.map_err(CoreError::Io)?.len();
    let start = if from > len { 0 } else { from };
    if start == len {
        return Ok((String::new(), start));
    }

    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(CoreError::Io)?;
    let mut bytes = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut bytes).await.map_err(CoreError::Io)?;

    let new_offset = start + bytes.len() as u64;
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
    fn the_first_three_bars_of_the_first_real_run_parse_exactly() {
        // Copied verbatim out of `train.log` of the first real training run on
        // this machine (FLUX.2 [klein] 4B, 2026-09-17, run
        // 01a0aeeb-f67a-7751-bad9-1da6e10f4df3). This is the ground truth the
        // parser is written against: tqdm emits the bar before it has a rate
        // or a postfix, then with a postfix but still no rate, and only then
        // a complete one -- so `?it/s` with no ETA and a missing postfix are
        // normal opening states, not malformed lines.
        let first = "myrender-v1:   0%|          | 0/600 [00:00<?, ?it/s]";
        let second =
            "myrender-v1:   0%|          | 0/600 [00:03<?, ?it/s, lr: 1.0e-04 loss: 7.309e-01]";
        let third = "myrender-v1:   0%|          | 1/600 [00:03<39:03,  3.91s/it, lr: 1.0e-04 loss: 7.309e-01]";

        let p1 = parse_run_progress(first, "myrender-v1").expect("bar 1");
        assert_eq!((p1.step, p1.total), (0, 600));
        assert_eq!(p1.eta_secs, None, "tqdm has no estimate yet");
        assert_eq!(p1.loss, None, "no postfix on the opening draw");
        assert_eq!(p1.lr, None);

        let p2 = parse_run_progress(second, "myrender-v1").expect("bar 2");
        assert_eq!((p2.step, p2.total), (0, 600));
        assert_eq!(p2.eta_secs, None);
        assert_eq!(p2.loss, Some(0.7309));
        assert_eq!(p2.lr, Some(1.0e-04));

        let p3 = parse_run_progress(third, "myrender-v1").expect("bar 3");
        assert_eq!((p3.step, p3.total), (1, 600));
        assert_eq!(p3.eta_secs, Some(39 * 60 + 3));
        assert_eq!(p3.loss, Some(0.7309));
        assert_eq!(p3.lr, Some(1.0e-04));
    }

    #[test]
    fn only_the_run_s_own_bar_counts_as_training_progress() {
        // Verbatim from the first real run (2026-09-17, run
        // 01a0aeeb-f67a-7751-bad9-1da6e10f4df3): ai-toolkit draws three other
        // tqdm bars before the training bar ever appears -- weight
        // quantisation, latent caching and the baseline sample generation.
        // They have the same shape, so a shape-only parser reads them as
        // training steps: the run showed "step 34" while the log still said
        // "Generating baseline samples before training", and each one would
        // have rewritten `total_steps` to its own total (25, 50, 2) too.
        let name = "myrender-v1";
        let quantising = "  0%|          | 0/25 [00:00<?, ?it/s]";
        let caching = "Caching Latents:  68%|######    | 34/50 [00:04<00:02,  7.2it/s]";
        let sampling = "Generating Samples: 100%|##########| 2/2 [00:35<00:00, 17.97s/it]";
        let training = "myrender-v1:   0%|          | 0/600 [00:00<?, ?it/s]";

        for other in [quantising, caching, sampling] {
            assert!(
                parse_progress(other).is_some(),
                "precondition: {other:?} has a progress-bar shape"
            );
            assert_eq!(
                parse_run_progress(other, name),
                None,
                "{other:?} is not this run's bar"
            );
        }

        let p = parse_run_progress(training, name).expect("the run's own bar must parse");
        assert_eq!(p.step, 0);
        assert_eq!(p.total, 600);
    }

    #[test]
    fn a_run_name_that_is_a_prefix_of_another_is_not_confused() {
        // `myrender` must not swallow `myrender-v1`'s bar, and the colon is
        // what separates the description from the bar.
        let line = "myrender-v1:  10%|#         | 60/600 [00:30<04:30,  2.0it/s]";
        assert_eq!(parse_run_progress(line, "myrender"), None);
        assert_eq!(
            parse_run_progress(line, "myrender-v1").map(|p| p.step),
            Some(60)
        );
    }

    #[test]
    fn a_run_name_with_colons_and_brackets_still_matches_its_own_bar() {
        let name = "my [lora]: run 1";
        let line = "my [lora]: run 1:  12%|##        | 240/2000 [01:02<07:35,  3.86it/s, lr: 1.0e-04 loss: 3.123e-01]";
        let p = parse_run_progress(line, name).expect("the run's own bar must parse");
        assert_eq!(p.step, 240);
        assert_eq!(p.total, 2000);
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
        // [`split_updates`] trims every update before handing it on, so the
        // completion line reaches this parser without its leading space.
        assert_eq!(parse_marker("- 1 completed job"), Some(Marker::Completed));
        assert_eq!(
            split_updates(
                "Result:
 - 1 completed job
"
            )
            .filter_map(parse_marker)
            .collect::<Vec<_>>(),
            vec![Marker::Completed],
            "the completion marker must survive the splitter the runner uses"
        );
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

    #[tokio::test]
    async fn tail_log_restarts_from_zero_after_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("log.txt");

        tokio::fs::write(&log_path, vec![b'a'; 100])
            .await
            .expect("write 100 bytes");
        let (_chunk, offset) = tail_log(&log_path, 0).await.expect("read from start");
        assert_eq!(offset, 100);

        // Simulate the log being truncated (or recreated) and written to
        // again — a shorter file at an offset the reader hasn't caught up
        // to yet, not "nothing new".
        let restarted: &[u8] = b"01234567890123456789";
        tokio::fs::write(&log_path, restarted)
            .await
            .expect("rewrite (truncate)");

        let (chunk, offset2) = tail_log(&log_path, offset)
            .await
            .expect("restart from zero after truncation");
        assert_eq!(chunk, std::str::from_utf8(restarted).unwrap());
        assert_eq!(offset2, restarted.len() as u64);
    }

    #[test]
    fn desc_with_brackets_still_parses() {
        let line = "my [job]:  12%|██        | 240/2000 [01:02<07:35,  3.86it/s, lr: 1.0e-04 loss: 3.123e-01]";
        let p = parse_progress(line).expect("should parse despite brackets in desc");
        assert_eq!(p.step, 240);
        assert_eq!(p.total, 2000);
    }

    #[test]
    fn look_alike_postfix_keys_do_not_shadow_loss() {
        let line = "job:  12%|██        | 240/2000 [01:02<07:35,  3.86it/s, avg_loss: 9.999 loss_2: 1.111 loss: 3.123e-01]";
        let p = parse_progress(line).expect("should parse");
        assert!((p.loss.expect("loss") - 0.3123).abs() < 1e-9);
    }

    #[test]
    fn a_partial_trailing_bar_is_skipped_and_the_last_complete_one_wins() {
        let complete =
            "job:  10%|█         | 10/100 [00:01<00:09,  10.0it/s, lr: 1.0e-04 loss: 1.000e+00]";
        // A `\r`-updated bar mid-write: no closing `]` yet.
        let partial = "job:  11%|█         | 11/100 [00:01<00:09,  10.0it/s, lr: 1.0e-04 loss: 9";
        let buf = format!("{complete}\r{partial}");

        let latest = latest_step_from_updates(&buf).expect("should find the last complete bar");
        assert_eq!(latest.step, 10);
    }
}
