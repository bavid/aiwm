//! Output retention: age- and/or size-based cleanup of `<outputs_dir>`.
//!
//! `docs/TODO.md` used to note that `about.outputs_bytes` + the Settings
//! "reveal" button only ever made the folder's size *visible* — there was no
//! automatic cleanup, size limit, or delete-old-outputs. This module is that
//! missing half: a [`RetentionPolicy`] (age and/or total-size caps, either or
//! both `0` to disable) plus [`sweep`], which enforces it against the flat
//! `<outputs_dir>` (`<job_id>.<ext>`, no subfolders).
//!
//! **Deliberately file-only, never the job's DB row.** A pruned job's
//! history, params and prompt stay in the Jobs list — only its rendered file
//! is gone. [`crate::api::handlers::job_output_path`] already treats a
//! missing/gone output file as "no output" (the same thing a job whose file
//! the user deleted by hand looks like today), so this is the simpler and
//! safer default: no risk of deleting a DB row out from under a UI that has
//! it open, no need to also touch `jobs.output_path`. The trade-off is a
//! gallery/job-detail entry that shows blank media after a sweep instead of
//! disappearing outright — acceptable, and reversible by just re-rendering.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

/// One regular file directly in the outputs folder.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputFile {
    path: PathBuf,
    bytes: u64,
    modified: OffsetDateTime,
}

/// The configured retention rules. Either field `0` disables that rule;
/// both `0` disables retention entirely (the historical, TODO-documented
/// behaviour — outputs live forever until deleted by hand).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionPolicy {
    /// Delete output files last modified more than this many days ago.
    pub max_age_days: u64,
    /// Once age-based deletion has run, keep the remaining total under this
    /// many MB — oldest files deleted first until it fits. Applied on top of
    /// (not instead of) the age rule, so both can be configured together.
    pub max_total_mb: u64,
}

impl RetentionPolicy {
    /// Whether either rule is actually configured — a manual "clean up now"
    /// with no policy set would otherwise silently do nothing, which is
    /// confusing; the caller uses this to grey the button out instead.
    pub fn is_active(&self) -> bool {
        self.max_age_days > 0 || self.max_total_mb > 0
    }
}

/// What one [`sweep`] run did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SweepResult {
    pub deleted_files: u64,
    pub freed_bytes: u64,
    /// One message per file that failed to delete — best-effort: a file the
    /// OS won't let go of right now (still open in a viewer, e.g.) doesn't
    /// abort the rest of the sweep.
    pub errors: Vec<String>,
}

/// The regular files directly in `dir` (never recurses — the outputs folder
/// is flat by construction). Missing dir, or an unreadable entry, just means
/// fewer/no candidates, not an error — the caller can't be blocked by that.
fn scan(dir: &Path) -> Vec<OutputFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified = meta.modified().ok()?;
            Some(OutputFile {
                path: e.path(),
                bytes: meta.len(),
                modified: OffsetDateTime::from(modified),
            })
        })
        .collect()
}

/// Which files `policy` marks for deletion, given `now` — split out from
/// [`sweep`] so the selection logic is testable without touching the
/// filesystem's write path.
fn files_to_delete(
    files: Vec<OutputFile>,
    policy: RetentionPolicy,
    now: OffsetDateTime,
) -> Vec<OutputFile> {
    let mut kept: Vec<OutputFile> = files;
    kept.sort_by_key(|f| f.modified); // oldest first, throughout

    let mut doomed: Vec<OutputFile> = Vec::new();
    if policy.max_age_days > 0 {
        let cutoff = now - Duration::days(policy.max_age_days.min(i64::MAX as u64) as i64);
        let (old, young): (Vec<_>, Vec<_>) = kept.into_iter().partition(|f| f.modified < cutoff);
        doomed.extend(old);
        kept = young;
    }

    if policy.max_total_mb > 0 {
        let budget_bytes = policy.max_total_mb.saturating_mul(1024 * 1024);
        let total: u64 = kept.iter().map(|f| f.bytes).sum();
        let mut over = total.saturating_sub(budget_bytes);
        let mut still_kept = Vec::with_capacity(kept.len());
        for f in kept {
            if over > 0 {
                over = over.saturating_sub(f.bytes);
                doomed.push(f);
            } else {
                still_kept.push(f);
            }
        }
        kept = still_kept;
    }

    let _ = kept; // only the doomed set is the caller's concern
    doomed
}

/// Enforce `policy` against `dir` right now: delete every file it marks,
/// best-effort. A no-op (returns the zero [`SweepResult`], touches nothing)
/// when `policy` has neither rule set. This is what both the manual
/// "clean up now" button and an optional startup sweep call.
pub fn sweep(dir: &Path, policy: RetentionPolicy) -> SweepResult {
    if !policy.is_active() {
        return SweepResult::default();
    }
    let doomed = files_to_delete(scan(dir), policy, OffsetDateTime::now_utc());

    let mut result = SweepResult::default();
    for f in doomed {
        match std::fs::remove_file(&f.path) {
            Ok(()) => {
                result.deleted_files += 1;
                result.freed_bytes = result.freed_bytes.saturating_add(f.bytes);
            }
            Err(e) => result.errors.push(format!("{}: {e}", f.path.display())),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration as StdDuration, SystemTime};

    fn touch(dir: &Path, name: &str, bytes: usize, age_days: u64) {
        let path = dir.join(name);
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
        let when = SystemTime::now() - StdDuration::from_secs(age_days * 86_400);
        // `File::open` is read-only, which Windows won't let `set_modified`
        // run against (`ERROR_ACCESS_DENIED`) -- open for write instead.
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn is_active_reflects_either_rule() {
        assert!(!RetentionPolicy::default().is_active());
        assert!(RetentionPolicy {
            max_age_days: 30,
            max_total_mb: 0
        }
        .is_active());
        assert!(RetentionPolicy {
            max_age_days: 0,
            max_total_mb: 10
        }
        .is_active());
    }

    #[test]
    fn sweep_is_a_noop_with_no_policy() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), "a.png", 100, 999);

        let result = sweep(tmp.path(), RetentionPolicy::default());

        assert_eq!(result, SweepResult::default());
        assert!(
            tmp.path().join("a.png").is_file(),
            "nothing should be touched"
        );
    }

    #[test]
    fn sweep_deletes_only_files_older_than_max_age_days() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), "old.png", 100, 40);
        touch(tmp.path(), "fresh.png", 100, 2);

        let result = sweep(
            tmp.path(),
            RetentionPolicy {
                max_age_days: 30,
                max_total_mb: 0,
            },
        );

        assert_eq!(result.deleted_files, 1);
        assert_eq!(result.freed_bytes, 100);
        assert!(result.errors.is_empty());
        assert!(!tmp.path().join("old.png").exists());
        assert!(tmp.path().join("fresh.png").exists());
    }

    #[test]
    fn sweep_trims_to_the_size_budget_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        // Three ~1.2 MB files, oldest to newest -- 3.6 MB total.
        touch(tmp.path(), "oldest.mp4", 1_200_000, 10);
        touch(tmp.path(), "middle.mp4", 1_200_000, 5);
        touch(tmp.path(), "newest.mp4", 1_200_000, 1);

        // Budget fits exactly two files (~2.29 MB) -- the single oldest must go.
        let result = sweep(
            tmp.path(),
            RetentionPolicy {
                max_age_days: 0,
                max_total_mb: 3,
            },
        );

        assert_eq!(result.deleted_files, 1);
        assert!(!tmp.path().join("oldest.mp4").exists());
        assert!(tmp.path().join("middle.mp4").exists());
        assert!(tmp.path().join("newest.mp4").exists());
    }

    #[test]
    fn sweep_applies_age_then_size_together() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), "ancient.mp4", 500_000, 100); // caught by age
        touch(tmp.path(), "old.mp4", 2_000_000, 20); // survives age, caught by size
        touch(tmp.path(), "fresh.mp4", 500_000, 1);

        let result = sweep(
            tmp.path(),
            RetentionPolicy {
                max_age_days: 30,
                max_total_mb: 1,
            },
        );

        assert!(!tmp.path().join("ancient.mp4").exists());
        assert!(!tmp.path().join("old.mp4").exists());
        assert!(tmp.path().join("fresh.mp4").exists());
        assert_eq!(result.deleted_files, 2);
    }

    #[test]
    fn sweep_on_a_missing_directory_is_a_harmless_noop() {
        let result = sweep(
            Path::new("Z:\\this\\does\\not\\exist"),
            RetentionPolicy {
                max_age_days: 1,
                max_total_mb: 0,
            },
        );
        assert_eq!(result, SweepResult::default());
    }

    #[test]
    fn files_to_delete_keeps_everything_under_budget() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), "a.png", 100, 1);
        touch(tmp.path(), "b.png", 100, 2);
        let files = scan(tmp.path());

        let doomed = files_to_delete(
            files,
            RetentionPolicy {
                max_age_days: 0,
                max_total_mb: 10,
            },
            OffsetDateTime::now_utc(),
        );
        assert!(doomed.is_empty());
    }
}
