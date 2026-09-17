//! Test fixture: a minimal `ai-toolkit` trainer stand-in.
//!
//! Reads the same `config.yaml` `core::training::config` renders, then speaks
//! the exact log dialect `core::training::progress` parses — a `tqdm` bar
//! redrawn with `\r`, `Saved checkpoint to …`, the resume markers, the OOM
//! block, the completion block — and writes the same on-disk layout
//! (`<training_folder>/<name>/<name>_<step:09>.safetensors`, `samples/`).
//! It exists so `core/tests/training_run.rs` can drive the real detached
//! launch, PID liveness, kill, resume and import cycle without a GPU or a
//! multi-GB Python install. Not part of the shipped product.
//!
//! Usage: `aiwm-fake-trainer <config.yaml> [-l <log>] [--oom-at N]
//! [--ms-per-step N]`. `-l` is ai-toolkit's own flag and tees every byte into
//! that file on top of stdout, exactly as the real trainer does (AIWM's
//! launcher redirects stdout into the same file, so lines legitimately appear
//! twice — the parsers are built for that).
//!
//! Fixture-only flags:
//! - `--oom-at N`      — at step `N`, print the OOM back-off block, the abort
//!   line and `Error running job: …`, then exit 1.
//! - `--ms-per-step N` — milliseconds per step (default 100).
//!
//! No signal handling on purpose: the app stops a run with `taskkill /T /F`,
//! so the fixture must die exactly that abruptly, leaving its last periodic
//! checkpoint as the only thing a resume can build on.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

/// A 1×1 transparent PNG — what a "generated sample" is here.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

const DEFAULT_MS_PER_STEP: u64 = 100;
/// Characters in the `tqdm` bar between the two `|`.
const BAR_WIDTH: usize = 10;
/// Stand-in for a real LoRA — the importer only hashes and copies bytes.
const CHECKPOINT_BYTES: &[u8] = b"aiwm-fake-trainer lora";

/// Exit with a message: a fixture given nonsense arguments or an unreadable
/// config has nothing useful to do, and a silent success would look like a
/// passing test.
fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("aiwm-fake-trainer: {msg}");
    std::process::exit(2)
}

// ------------------------------------------------------------------ config

/// Only the handful of keys this fixture reacts to; every other key in the
/// rendered config is ignored (serde skips unknown fields).
#[derive(Debug, Deserialize)]
struct ConfigFile {
    config: JobConfig,
}

#[derive(Debug, Deserialize)]
struct JobConfig {
    name: String,
    process: Vec<ProcessBlock>,
}

#[derive(Debug, Deserialize)]
struct ProcessBlock {
    training_folder: String,
    save: SaveBlock,
    train: TrainBlock,
    sample: SampleBlock,
}

#[derive(Debug, Deserialize)]
struct SaveBlock {
    save_every: u64,
}

#[derive(Debug, Deserialize)]
struct TrainBlock {
    steps: u64,
    #[serde(default)]
    lr: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SampleBlock {
    sample_every: u64,
    #[serde(default)]
    prompts: Vec<String>,
}

// -------------------------------------------------------------------- args

#[derive(Debug)]
struct Args {
    config: PathBuf,
    log: Option<PathBuf>,
    oom_at: Option<u64>,
    step_delay: Duration,
}

fn parse_args() -> Args {
    let mut config: Option<PathBuf> = None;
    let mut log = None;
    let mut oom_at = None;
    let mut ms_per_step = DEFAULT_MS_PER_STEP;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-l" => log = Some(PathBuf::from(next_value(&mut args, "-l"))),
            "--oom-at" => oom_at = Some(parse_u64(&next_value(&mut args, "--oom-at"), "--oom-at")),
            "--ms-per-step" => {
                ms_per_step = parse_u64(&next_value(&mut args, "--ms-per-step"), "--ms-per-step");
            }
            other if other.starts_with('-') => die(format!("unknown flag {other}")),
            other if config.is_none() => config = Some(PathBuf::from(other)),
            other => die(format!("unexpected extra argument {other}")),
        }
    }

    Args {
        config: config.unwrap_or_else(|| die("missing the config.yaml path")),
        log,
        oom_at,
        step_delay: Duration::from_millis(ms_per_step),
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next()
        .unwrap_or_else(|| die(format!("{flag} needs a value")))
}

fn parse_u64(raw: &str, flag: &str) -> u64 {
    raw.parse()
        .unwrap_or_else(|_| die(format!("{flag} needs a whole number, got {raw:?}")))
}

// ------------------------------------------------------------------ output

/// stdout plus, when `-l` was given, the log file — flushed after every write
/// so a poller reading the file sees each bar as it is drawn.
struct Out {
    log: Option<File>,
}

impl Out {
    fn open(path: Option<&Path>) -> Self {
        let log = path.map(|p| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .unwrap_or_else(|e| die(format!("cannot open the log file {}: {e}", p.display())))
        });
        Self { log }
    }

    fn write(&mut self, text: &str) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
        if let Some(file) = &mut self.log {
            let _ = file.write_all(text.as_bytes());
            let _ = file.flush();
        }
    }

    fn line(&mut self, text: &str) {
        self.write(&format!("{text}\n"));
    }
}

// --------------------------------------------------------------- formatting

/// `3.123e-01` — Python's two-digit, always-signed exponent, which Rust's
/// `{:e}` does not produce on its own.
fn sci(value: f64, decimals: usize) -> String {
    let raw = format!("{value:.decimals$e}");
    let Some((mantissa, exponent)) = raw.split_once('e') else {
        return raw;
    };
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(rest) => ('-', rest),
        None => ('+', exponent.trim_start_matches('+')),
    };
    format!("{mantissa}e{sign}{digits:0>2}")
}

/// `mm:ss`, or `h:mm:ss` once it no longer fits — tqdm's own elapsed/remaining
/// format.
fn clock(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// One redrawn `tqdm` line, e.g.
/// `lifecycle:  12%|█         | 30/250 [00:03<00:22,  9.98it/s, lr: 1.0e-04 loss: 4.4e-01]`.
fn progress_line(name: &str, step: u64, total: u64, lr: f64, ms_per_step: u64) -> String {
    let pct = (step * 100).checked_div(total).unwrap_or(100);
    let filled = (pct as usize * BAR_WIDTH / 100).min(BAR_WIDTH);
    let bar = format!("{}{}", "█".repeat(filled), " ".repeat(BAR_WIDTH - filled));
    let per_step = ms_per_step.max(1);
    let elapsed = clock(step * per_step / 1000);
    let remaining = clock(total.saturating_sub(step) * per_step / 1000);
    let rate = 1000.0 / per_step as f64;
    // Plausibly decaying, never zero — the poller records it as `last_loss`.
    let loss = 0.5 * (-(step as f64) / (total.max(1) as f64 * 0.6)).exp();
    format!(
        "\r{name}: {pct:>3}%|{bar}| {step}/{total} [{elapsed}<{remaining}, {rate:>5.2}it/s, \
         lr: {} loss: {}]",
        sci(lr, 1),
        sci(loss, 3)
    )
}

// ------------------------------------------------------------------ on disk

fn checkpoint_path(run_dir: &Path, name: &str, step: u64) -> PathBuf {
    run_dir.join(format!("{name}_{step:09}.safetensors"))
}

/// The highest `<name>_<step:09>.safetensors` already in `run_dir` — what
/// ai-toolkit's auto-resume picks up when the same config runs again.
fn latest_checkpoint(run_dir: &Path, name: &str) -> Option<(u64, PathBuf)> {
    let entries = fs::read_dir(run_dir).ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(step) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".safetensors"))
            .and_then(|stem| stem.strip_prefix(name))
            .and_then(|rest| rest.strip_prefix('_'))
            .and_then(|digits| digits.parse::<u64>().ok())
        else {
            continue;
        };
        if best.as_ref().is_none_or(|(seen, _)| step > *seen) {
            best = Some((step, path));
        }
    }
    best
}

fn write_checkpoint(out: &mut Out, run_dir: &Path, name: &str, step: u64) {
    let path = checkpoint_path(run_dir, name, step);
    if let Err(e) = fs::write(&path, CHECKPOINT_BYTES) {
        die(format!("cannot write {}: {e}", path.display()));
    }
    out.line(&format!("Saved checkpoint to {}", path.display()));
}

fn write_samples(out: &mut Out, run_dir: &Path, step: u64, prompts: usize) {
    let dir = run_dir.join("samples");
    if let Err(e) = fs::create_dir_all(&dir) {
        die(format!("cannot create {}: {e}", dir.display()));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    for index in 0..prompts {
        let path = dir.join(format!("{stamp}_{step:09}_{index}.png"));
        if let Err(e) = fs::write(&path, TINY_PNG) {
            die(format!("cannot write {}: {e}", path.display()));
        }
    }
    // Deliberately bracket-free: `parse_progress` anchors on a trailing
    // `[…]`, so a sample notice must not look like a step update.
    out.line(&format!(
        "Generating Images: {prompts} sample(s) for step {step:09}"
    ));
}

// --------------------------------------------------------------------- main

/// The job name and its single process block — or an exit with the reason.
fn load_config(path: &Path) -> (String, ProcessBlock) {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| die(format!("cannot read the config {}: {e}", path.display())));
    let parsed: ConfigFile = serde_yaml_ng::from_str(&raw)
        .unwrap_or_else(|e| die(format!("cannot parse the config: {e}")));
    let job = parsed.config;
    let process = job
        .process
        .into_iter()
        .next()
        .unwrap_or_else(|| die("the config has no process block"));
    (job.name, process)
}

/// ai-toolkit's auto-resume: re-running the same config name over an output
/// directory that already holds checkpoints continues from the newest one and
/// says so in the two lines `progress::parse_marker` looks for.
fn resume_from(out: &mut Out, run_dir: &Path, name: &str) -> u64 {
    match latest_checkpoint(run_dir, name) {
        Some((found, path)) => {
            out.line(&format!(
                "#### IMPORTANT RESUMING FROM {} ####",
                path.display()
            ));
            out.line(&format!(
                "Found step {found} in metadata, starting from there"
            ));
            found
        }
        None => 0,
    }
}

fn main() {
    let args = parse_args();
    let (name, process) = load_config(&args.config);

    let run_dir = Path::new(&process.training_folder).join(&name);
    if let Err(e) = fs::create_dir_all(&run_dir) {
        die(format!("cannot create {}: {e}", run_dir.display()));
    }

    let total = process.train.steps;
    let save_every = process.save.save_every.max(1);
    let sample_every = process.sample.sample_every.max(1);
    let prompts = process.sample.prompts.len();
    let lr = process.train.lr.unwrap_or(1e-4);
    let ms_per_step = args.step_delay.as_millis().min(u128::from(u64::MAX)) as u64;

    let mut out = Out::open(args.log.as_deref());
    out.line("Running 1 job");

    let mut step = resume_from(&mut out, &run_dir, &name);

    let mut last_saved = step;
    while step < total {
        std::thread::sleep(args.step_delay);
        step += 1;

        if args.oom_at == Some(step) {
            for attempt in 1..=3 {
                out.line(&format!(
                    "# OOM during training step, skipping batch {attempt}/3 #"
                ));
            }
            out.line("OOM during training step 3 times in a row, aborting training");
            out.line(
                "Error running job: OOM during training step 3 times in a row, aborting training",
            );
            std::process::exit(1);
        }

        out.write(&progress_line(&name, step, total, lr, ms_per_step));

        if step % save_every == 0 {
            write_checkpoint(&mut out, &run_dir, &name, step);
            last_saved = step;
        }
        if step % sample_every == 0 {
            write_samples(&mut out, &run_dir, step, prompts);
        }
    }

    if last_saved != step {
        write_checkpoint(&mut out, &run_dir, &name, step);
    }
    out.write("\n========================================\nResult:\n - 1 completed job\n========================================\n");
}
