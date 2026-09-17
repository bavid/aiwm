//! The pinned base-weight manifests (spec §5 "Echter Lauf", plan Task 11):
//! for each trainable family, the upstream repo, where its snapshot belongs
//! under the model store, which files the download should select, and — where
//! a real download has actually happened on a real machine — the exact size
//! and SHA-256 of every file the trainer reads.
//!
//! "Every file the trainer reads" is narrower than "every file in the repo",
//! and for the FLUX.2 [klein] sizes it is dramatically narrower: one blob out
//! of twenty-odd files. What a trainer opens is a property of the *trainer*,
//! not of the repo layout, so each entry here is derived from the pinned
//! `ai-toolkit` commit's loader and confirmed by a real run — see the
//! required-files note in [`super::profile`].
//!
//! This is the *integrity* half of a base checkpoint. The *completeness*
//! half lives next door in [`super::profile::find_staged_base`]: which
//! directory holds all of [`super::profile::BaseWeight::required_files`].
//! Preflight runs completeness first and [`verify_base_dir`] second, so a
//! half-downloaded folder is reported as "incomplete — download again" and a
//! complete-but-corrupt one as the file that is wrong, instead of both
//! surfacing an hour into a run as an unreadable tensor.
//!
//! **Hashes are only ever pinned from a download performed and hashed on
//! this machine.** An entry for a repo nobody has downloaded yet carries
//! `files: &[]` — an honest "nothing pinned" rather than a value copied from
//! a web page. [`verify_base_dir`] treats such an entry as "completeness is
//! all we can check", never as "verified".

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{training_err, training_refusal};
use crate::Result;

/// One file inside a base snapshot, pinned by size and content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct BaseFile {
    /// Relative to the snapshot directory, forward slashes.
    pub path: &'static str,
    pub size_bytes: u64,
    /// Lower-case hex SHA-256, computed locally — never transcribed.
    pub sha256: &'static str,
}

/// The download-and-verify manifest for one trainable family's base weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TrainingBase {
    /// Matches [`super::profile::TrainingProfile::family`].
    pub family: &'static str,
    pub repo: &'static str,
    /// Where the snapshot belongs under the model store, forward slashes.
    pub local_subdir: &'static str,
    /// The `hf download` flags that select what to fetch, already quoted,
    /// appended verbatim to the command — `["--include", "\"x.safetensors\""]`
    /// or a list of `--exclude` patterns.
    ///
    /// Selector flags rather than a bare exclusion list because for the two
    /// FLUX.2 [klein] sizes the trainer opens exactly *one* file out of a
    /// 16–30 GB repo (see [`super::profile`]'s required-files note), and
    /// "everything except these twenty things" is both longer and easier to
    /// get wrong than "this one thing".
    pub download_filters: &'static [&'static str],
    /// Empty until a real download on a real machine has been hashed.
    pub files: &'static [BaseFile],
}

/// The one file the FLUX.2 [klein] 4B trainer opens under `name_or_path`,
/// pinned from the download on this machine (2026-09-17) with `sha256sum`.
/// Agreement with the Hub's published blob digest is the cross-check, not the
/// source.
///
/// It is *one* file, not the five of the diffusers layout — see the
/// required-files note in [`super::profile`], which a failed real run
/// established the hard way. The trainer's other two inputs (the Qwen3 text
/// encoder and the VAE) are separate Hub repos it fetches itself, so they
/// are not part of this snapshot and cannot be pinned here.
const FLUX2_KLEIN_4B_FILES: &[BaseFile] = &[BaseFile {
    path: "flux-2-klein-base-4b.safetensors",
    size_bytes: 7_751_105_712,
    sha256: "9c5fed22b76baea749d88fc2abe3ad53245e7b21a0d353a762665eea00043b92",
}];

/// One entry per family in [`super::profile::PROFILES`], so the Training tab
/// can show a download command for any of them.
///
/// Only the 4B carries pinned `files`: it is the only base that has been
/// downloaded and hashed here. The other three are the repo/selector/target
/// half of the manifest with `files: &[]` — hashes get pinned after the first
/// verified download, one family at a time.
pub const TRAINING_BASES: &[TrainingBase] = &[
    TrainingBase {
        family: "flux2-klein-4b",
        repo: "black-forest-labs/FLUX.2-klein-base-4B",
        local_subdir: "training/flux2-klein-4b",
        // One file out of a 23 GB repo. `--include` rather than a long
        // `--exclude` list because the trainer opens exactly this blob and
        // nothing else locally -- verified by reading the pinned commit's
        // loader and by a real run that failed without it.
        download_filters: &["--include", "\"flux-2-klein-base-4b.safetensors\""],
        files: FLUX2_KLEIN_4B_FILES,
    },
    TrainingBase {
        family: "flux2-klein-9b",
        repo: "black-forest-labs/FLUX.2-klein-base-9B",
        local_subdir: "training/flux2-klein-9b",
        download_filters: &["--include", "\"flux-2-klein-base-9b.safetensors\""],
        // Hashes pinned after the first verified download.
        files: &[],
    },
    TrainingBase {
        family: "sdxl",
        repo: "stabilityai/stable-diffusion-xl-base-1.0",
        local_subdir: "training/sdxl",
        download_filters: &["--exclude", "\"*.jpg\""],
        // Hashes pinned after the first verified download.
        files: &[],
    },
    TrainingBase {
        family: "wan",
        repo: "Wan-AI/Wan2.2-TI2V-5B-Diffusers",
        local_subdir: "training/wan",
        download_filters: &["--exclude", "\"*.jpg\""],
        // Hashes pinned after the first verified download.
        files: &[],
    },
];

/// The manifest for a [`super::profile::TrainingProfile::family`].
pub fn find_base(family: &str) -> Option<&'static TrainingBase> {
    TRAINING_BASES.iter().find(|b| b.family == family)
}

impl TrainingBase {
    /// Where this base's snapshot belongs under `store_root`, with the
    /// platform's own separators throughout (a `training/flux2-klein-4b`
    /// literal joined onto `E:\AI\models` would otherwise produce a
    /// mixed-separator path nobody can paste into a shell).
    pub fn local_dir(&self, store_root: &Path) -> PathBuf {
        let mut dir = store_root.to_path_buf();
        for part in self.local_subdir.split('/').filter(|p| !p.is_empty()) {
            dir.push(part);
        }
        dir
    }
}

/// The exact `hf` CLI line that stages this base under `store_root` — the
/// one the Training tab's preflight shows for copying, built here so the UI
/// cannot drift from what [`verify_base_dir`] then expects to find.
pub fn hf_download_command(base: &TrainingBase, store_root: &Path) -> String {
    let mut cmd = format!(
        "hf download {} --local-dir {}",
        base.repo,
        base.local_dir(store_root).display()
    );
    for flag in base.download_filters {
        cmd.push(' ');
        cmd.push_str(flag);
    }
    cmd
}

/// Check a staged snapshot against its pinned manifest.
///
/// `full = false` (what preflight runs before every start) checks that every
/// pinned file is present at its pinned size. `full = true` also hashes each
/// one.
///
/// The split is about what each caller can afford, not about thoroughness for
/// its own sake. A klein base is a *single* 7.7–18 GB blob, so "hash one small
/// file as a spot check" is not available: there is nothing small to hash.
/// Hashing on every Start would stall the button for the better part of a
/// minute on work that has not changed since the last run, so the full check
/// runs once, deliberately, when the snapshot is registered
/// (`core/tests/register_training_base.rs`), and the start path checks sizes —
/// which is what actually catches the realistic failure, a truncated or
/// interrupted download.
///
/// A base with no pinned files verifies trivially: there is nothing to check
/// beyond the completeness [`super::profile::find_staged_base`] already
/// established. Errors are refusals — every one of them names the file and
/// says to download the repo again.
pub fn verify_base_dir(dir: &Path, base: &TrainingBase, full: bool) -> Result<()> {
    for file in base.files {
        let path = dir.join(file.path);
        let meta = std::fs::metadata(&path).map_err(|e| {
            training_refusal(format!(
                "{} is missing from the base weights for {} ({e}) — download {} again",
                file.path, base.family, base.repo
            ))
        })?;
        if meta.len() != file.size_bytes {
            return Err(training_refusal(format!(
                "{} in the base weights for {} is {} bytes, expected {} — download {} again",
                file.path,
                base.family,
                meta.len(),
                file.size_bytes,
                base.repo
            )));
        }
        if !full {
            continue;
        }
        let actual = sha256_file(&path)?;
        if actual != file.sha256 {
            return Err(training_refusal(format!(
                "{} in the base weights for {} does not match its pinned checksum \
                 (got {actual}) — download {} again",
                file.path, base.family, base.repo
            )));
        }
    }
    Ok(())
}

/// Streaming SHA-256 of a file, lower-case hex. A read failure here is a
/// fault, not a refusal: the file was there and the right size a moment ago.
fn sha256_file(path: &Path) -> Result<String> {
    const CHUNK: usize = 1024 * 1024;
    let mut file = std::fs::File::open(path)
        .map_err(|e| training_err(format!("cannot read {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| training_err(format!("cannot read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::profile::PROFILES;

    /// A manifest over two small files whose hashes are computed here, so the
    /// fixtures can be written into a tempdir instead of needing 16 GB of
    /// real weights. Mirrors the shape of the pinned 4B entry: one
    /// `.safetensors` that is the quick-hash target (the smaller one) and one
    /// that is not.
    const SMALL: &str = "small/vae.safetensors";
    const LARGE: &str = "large/transformer.safetensors";

    /// sha256("vae") and sha256("transformer-weights"), computed by the test
    /// itself in [`fixture`] — pinned here only so the manifest is `'static`.
    const SMALL_SHA: &str = "e089e84942af6414b066654f297dfbb11bf84b3e8b46d7252710076aff68d122";
    const LARGE_SHA: &str = "6fd98e549137a13143a24c28c68b91c803c29a0b3f21fe3ce6071719fd2e2896";

    const FIXTURE_FILES: &[BaseFile] = &[
        BaseFile {
            path: LARGE,
            size_bytes: 19,
            sha256: LARGE_SHA,
        },
        BaseFile {
            path: SMALL,
            size_bytes: 3,
            sha256: SMALL_SHA,
        },
    ];

    const FIXTURE: TrainingBase = TrainingBase {
        family: "fixture",
        repo: "acme/fixture-base",
        local_subdir: "training/fixture",
        download_filters: &["--exclude", "\"*.jpg\""],
        files: FIXTURE_FILES,
    };

    const EMPTY_FIXTURE: TrainingBase = TrainingBase {
        family: "fixture-empty",
        repo: "acme/unhashed-base",
        local_subdir: "training/fixture-empty",
        download_filters: &["--exclude", "\"*.jpg\""],
        files: &[],
    };

    fn write(dir: &Path, relative: &str, bytes: &[u8]) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, bytes).expect("write fixture file");
    }

    /// A tempdir holding a snapshot that matches [`FIXTURE`] exactly.
    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(tmp.path(), SMALL, b"vae");
        write(tmp.path(), LARGE, b"transformer-weights");
        // The manifest's `'static` hashes really are of this content.
        assert_eq!(
            sha256_file(&tmp.path().join(SMALL)).expect("hash small"),
            SMALL_SHA
        );
        assert_eq!(
            sha256_file(&tmp.path().join(LARGE)).expect("hash large"),
            LARGE_SHA
        );
        tmp
    }

    #[test]
    fn a_matching_snapshot_verifies_quick_and_full() {
        let tmp = fixture();
        assert!(verify_base_dir(tmp.path(), &FIXTURE, false).is_ok());
        assert!(verify_base_dir(tmp.path(), &FIXTURE, true).is_ok());
    }

    #[test]
    fn a_missing_file_is_refused_by_name() {
        let tmp = fixture();
        std::fs::remove_file(tmp.path().join(LARGE)).expect("remove");
        let err = verify_base_dir(tmp.path(), &FIXTURE, false)
            .expect_err("a missing file must not verify");
        let msg = err.to_string();
        assert!(msg.contains(LARGE), "message must name the file: {msg}");
        assert!(
            msg.contains("acme/fixture-base"),
            "message must say what to download again: {msg}"
        );
    }

    #[test]
    fn a_size_mismatch_is_refused_by_name_even_in_quick_mode() {
        let tmp = fixture();
        // Same first bytes, wrong length: only the size check can see this
        // without hashing 7.7 GB, which is exactly what the start path's
        // cheap check is for.
        write(tmp.path(), LARGE, b"transformer-weights-and-then-some");
        let err = verify_base_dir(tmp.path(), &FIXTURE, false)
            .expect_err("a truncated/extended file must not verify");
        let msg = err.to_string();
        assert!(msg.contains(LARGE), "message must name the file: {msg}");
        assert!(
            msg.contains("19"),
            "message must give the expected size: {msg}"
        );
    }

    #[test]
    fn a_hash_mismatch_at_the_right_size_is_refused_by_the_full_check() {
        let tmp = fixture();
        write(tmp.path(), SMALL, b"VAE"); // same 3 bytes, different content
        let err = verify_base_dir(tmp.path(), &FIXTURE, true)
            .expect_err("corrupt content must not verify");
        let msg = err.to_string();
        assert!(msg.contains(SMALL), "message must name the file: {msg}");
        assert!(
            msg.contains("checksum"),
            "message must say it is a checksum problem: {msg}"
        );
    }

    #[test]
    fn the_quick_check_reads_sizes_only_and_the_full_check_reads_content() {
        let tmp = fixture();
        // Corrupt both files without changing either length. The start path
        // must not pay to notice -- it would be re-reading gigabytes that
        // have not changed since the snapshot was registered and fully
        // verified -- and the full check must notice.
        write(tmp.path(), LARGE, b"TRANSFORMER-WEIGHTS");
        write(tmp.path(), SMALL, b"VAE");
        assert!(
            verify_base_dir(tmp.path(), &FIXTURE, false).is_ok(),
            "the quick check must not read file contents"
        );
        let err = verify_base_dir(tmp.path(), &FIXTURE, true)
            .expect_err("the full check must hash every file");
        assert!(err.to_string().contains("checksum"));
    }

    #[test]
    fn a_base_without_pinned_hashes_verifies_trivially() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(verify_base_dir(tmp.path(), &EMPTY_FIXTURE, false).is_ok());
        assert!(verify_base_dir(tmp.path(), &EMPTY_FIXTURE, true).is_ok());
    }

    #[test]
    fn the_download_command_names_the_repo_the_target_and_the_selector() {
        // This is the command that was actually run on 2026-09-17 to stage
        // the 4B (modulo the store root), and the file it fetched is the one
        // pinned in `FLUX2_KLEIN_4B_FILES`.
        let base = find_base("flux2-klein-4b").expect("4B base");
        let store = Path::new(if cfg!(windows) {
            r"E:\AI\models"
        } else {
            "/srv/models"
        });
        let target = if cfg!(windows) {
            r"E:\AI\models\training\flux2-klein-4b"
        } else {
            "/srv/models/training/flux2-klein-4b"
        };
        assert_eq!(
            hf_download_command(base, store),
            format!(
                "hf download black-forest-labs/FLUX.2-klein-base-4B --local-dir {target} \
                 --include \"flux-2-klein-base-4b.safetensors\""
            )
        );
    }

    #[test]
    fn the_klein_selectors_fetch_exactly_the_file_the_profile_requires() {
        // The download command and the completeness check must name the same
        // file: a selector that fetches something `find_staged_base` does not
        // look for produces a download that can never satisfy preflight.
        for family in ["flux2-klein-4b", "flux2-klein-9b"] {
            let base = find_base(family).expect("klein base");
            let required = crate::training::profile::find_for_family(family)
                .expect("klein profile")
                .base
                .required_files;
            assert_eq!(required.len(), 1, "{family}: one required file");
            let quoted = format!("\"{}\"", required[0]);
            assert!(
                base.download_filters.contains(&"--include"),
                "{family}: the selector must be an include"
            );
            assert!(
                base.download_filters.contains(&quoted.as_str()),
                "{family}: the selector must name {}, got {:?}",
                required[0],
                base.download_filters
            );
        }
    }

    #[test]
    fn local_dir_uses_platform_separators_throughout() {
        let base = find_base("flux2-klein-4b").expect("4B base");
        let dir = base.local_dir(Path::new(if cfg!(windows) {
            r"E:\AI\models"
        } else {
            "/srv/models"
        }));
        assert!(dir.ends_with("flux2-klein-4b"));
        if cfg!(windows) {
            assert!(
                !dir.display().to_string().contains('/'),
                "a Windows path must not carry a forward slash: {}",
                dir.display()
            );
        }
    }

    #[test]
    fn every_profile_family_has_a_base_manifest() {
        for profile in PROFILES {
            let base = find_base(profile.family)
                .unwrap_or_else(|| panic!("no base manifest for {}", profile.family));
            assert_eq!(
                base.repo, profile.base.repo,
                "{}: the manifest and the profile must name the same repo",
                profile.family
            );
        }
    }

    #[test]
    fn pinned_hashes_are_lower_case_hex_and_paths_are_relative() {
        for base in TRAINING_BASES {
            for file in base.files {
                assert_eq!(
                    file.sha256.len(),
                    64,
                    "{}/{}: a SHA-256 is 64 hex characters",
                    base.family,
                    file.path
                );
                assert!(
                    file.sha256
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                    "{}/{}: hashes are pinned lower-case",
                    base.family,
                    file.path
                );
                assert!(
                    !file.path.starts_with('/') && !file.path.contains('\\'),
                    "{}/{}: paths are relative with forward slashes",
                    base.family,
                    file.path
                );
                assert!(file.size_bytes > 0, "{}/{}: size", base.family, file.path);
            }
        }
    }

    #[test]
    fn the_4b_manifest_pins_every_file_the_profile_requires() {
        let base = find_base("flux2-klein-4b").expect("4B base");
        let pinned: Vec<&str> = base.files.iter().map(|f| f.path).collect();
        for required in crate::training::profile::find_for_family("flux2-klein-4b")
            .expect("4B profile")
            .base
            .required_files
        {
            assert!(
                pinned.contains(required),
                "{required} is required but not pinned"
            );
        }
        assert_eq!(
            base.files.len(),
            1,
            "the trainer opens exactly one local file for the 4B"
        );
    }
}
