//! The pinned base-weight manifests (spec §5 "Echter Lauf", plan Task 11):
//! for each trainable family, the upstream repo, where its snapshot belongs
//! under the model store, what the download must leave out, and — where a
//! real download has actually happened on a real machine — the exact size
//! and SHA-256 of every file the trainer reads.
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
    /// `hf download --exclude` patterns: sample images and single-file
    /// duplicates of weights the diffusers layout already carries.
    pub exclude: &'static [&'static str],
    /// Empty until a real download on a real machine has been hashed.
    pub files: &'static [BaseFile],
}

/// The five files the FLUX.2 [klein] trainer actually reads, pinned from the
/// download on this machine (2026-09-17). Every hash here was produced by
/// `sha256sum` over the staged snapshot; they happen to agree with the Hub's
/// published blob digests, which is the cross-check, not the source.
const FLUX2_KLEIN_4B_FILES: &[BaseFile] = &[
    BaseFile {
        path: "transformer/diffusion_pytorch_model.safetensors",
        size_bytes: 7_751_109_744,
        sha256: "e109674697ffa1a3983126e32512f5428a9442bd8df59f9c95566ee90a473bb6",
    },
    BaseFile {
        path: "text_encoder/model-00001-of-00002.safetensors",
        size_bytes: 4_967_215_360,
        sha256: "8c0506e7f4936fa7e26183a4fd8da4e2bdbc5990ba64ae441f965d51228f36ea",
    },
    BaseFile {
        path: "text_encoder/model-00002-of-00002.safetensors",
        size_bytes: 3_077_766_632,
        sha256: "82f2bd839378541b0557bfabaf37c7d3d637071fdcb73302dedd7cf61162ce07",
    },
    BaseFile {
        path: "vae/diffusion_pytorch_model.safetensors",
        size_bytes: 168_120_878,
        sha256: "ca70d2202afe6415bdbcb8793ba8cd99fd159cfe6192381504d6c4d3036e0f04",
    },
    BaseFile {
        path: "tokenizer/tokenizer.json",
        size_bytes: 11_422_654,
        sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    },
];

/// One entry per family in [`super::profile::PROFILES`], so the Training tab
/// can show a download command for any of them.
///
/// Only the 4B carries pinned `files`: it is the only base that has been
/// downloaded and hashed here. The other three are the repo/exclude/target
/// half of the manifest with `files: &[]` — hashes get pinned after the first
/// verified download, one family at a time.
pub const TRAINING_BASES: &[TrainingBase] = &[
    TrainingBase {
        family: "flux2-klein-4b",
        repo: "black-forest-labs/FLUX.2-klein-base-4B",
        local_subdir: "training/flux2-klein-4b",
        // `*.jpg` are the three README sample images; the single-file
        // `.safetensors` is a duplicate of `transformer/` in one blob, 7.7 GB
        // the trainer never opens.
        exclude: &["*.jpg", "flux-2-klein-base-4b.safetensors"],
        files: FLUX2_KLEIN_4B_FILES,
    },
    TrainingBase {
        family: "flux2-klein-9b",
        repo: "black-forest-labs/FLUX.2-klein-base-9B",
        local_subdir: "training/flux2-klein-9b",
        exclude: &["*.jpg", "flux-2-klein-base-9b.safetensors"],
        // Hashes pinned after the first verified download.
        files: &[],
    },
    TrainingBase {
        family: "sdxl",
        repo: "stabilityai/stable-diffusion-xl-base-1.0",
        local_subdir: "training/sdxl",
        exclude: &["*.jpg"],
        // Hashes pinned after the first verified download.
        files: &[],
    },
    TrainingBase {
        family: "wan",
        repo: "Wan-AI/Wan2.2-TI2V-5B-Diffusers",
        local_subdir: "training/wan",
        exclude: &["*.jpg"],
        // Hashes pinned after the first verified download.
        files: &[],
    },
];

/// The manifest for a [`super::profile::TrainingProfile::family`].
pub fn find_base(family: &str) -> Option<&'static TrainingBase> {
    TRAINING_BASES.iter().find(|b| b.family == family)
}

impl TrainingBase {
    /// The one file a quick verification hashes: the smallest pinned
    /// `.safetensors`. Size checks already cover every file; this adds
    /// content verification on real weights without reading the 7.7 GB
    /// transformer on every Start. For the 4B that is the 168 MB VAE.
    fn quick_hash_target(&self) -> Option<&BaseFile> {
        self.files
            .iter()
            .filter(|f| f.path.ends_with(".safetensors"))
            .min_by_key(|f| f.size_bytes)
    }

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
    for pattern in base.exclude {
        cmd.push_str(&format!(" --exclude \"{pattern}\""));
    }
    cmd
}

/// Check a staged snapshot against its pinned manifest.
///
/// `full = false` (what preflight runs before every start) checks the size of
/// every pinned file and the SHA-256 of one of them — see
/// [`TrainingBase::quick_hash_target`]. `full = true` hashes all of them,
/// which for the 4B means reading 16 GB, so it belongs behind an explicit
/// "verify base weights" action rather than in the start path.
///
/// A base with no pinned files verifies trivially: there is nothing to check
/// beyond the completeness [`super::profile::find_staged_base`] already
/// established. Errors are refusals — every one of them names the file and
/// says to download the repo again.
pub fn verify_base_dir(dir: &Path, base: &TrainingBase, full: bool) -> Result<()> {
    let quick_target = base.quick_hash_target().map(|f| f.path);
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
        if !full && Some(file.path) != quick_target {
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
        exclude: &["*.jpg"],
        files: FIXTURE_FILES,
    };

    const EMPTY_FIXTURE: TrainingBase = TrainingBase {
        family: "fixture-empty",
        repo: "acme/unhashed-base",
        local_subdir: "training/fixture-empty",
        exclude: &["*.jpg"],
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
        // without hashing 7.7 GB, which is exactly why quick mode checks
        // every file's size and not just the one it hashes.
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
    fn a_hash_mismatch_at_the_right_size_is_refused() {
        let tmp = fixture();
        write(tmp.path(), SMALL, b"VAE"); // same 3 bytes, different content
        let err = verify_base_dir(tmp.path(), &FIXTURE, false)
            .expect_err("corrupt content must not verify");
        let msg = err.to_string();
        assert!(msg.contains(SMALL), "message must name the file: {msg}");
        assert!(
            msg.contains("checksum"),
            "message must say it is a checksum problem: {msg}"
        );
    }

    #[test]
    fn quick_mode_hashes_only_the_smallest_weights_file() {
        let tmp = fixture();
        // Corrupt the large file without changing its length: quick mode
        // cannot see it, full mode must.
        write(tmp.path(), LARGE, b"TRANSFORMER-WEIGHTS");
        assert!(
            verify_base_dir(tmp.path(), &FIXTURE, false).is_ok(),
            "quick mode must not read the large file"
        );
        let err = verify_base_dir(tmp.path(), &FIXTURE, true)
            .expect_err("full mode must hash every file");
        assert!(err.to_string().contains(LARGE));
    }

    #[test]
    fn a_base_without_pinned_hashes_verifies_trivially() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(verify_base_dir(tmp.path(), &EMPTY_FIXTURE, false).is_ok());
        assert!(verify_base_dir(tmp.path(), &EMPTY_FIXTURE, true).is_ok());
    }

    #[test]
    fn the_download_command_names_the_repo_target_and_every_exclusion() {
        let base = find_base("flux2-klein-4b").expect("4B base");
        let store = Path::new(if cfg!(windows) {
            r"E:\AI\models"
        } else {
            "/srv/models"
        });
        let expected = if cfg!(windows) {
            "hf download black-forest-labs/FLUX.2-klein-base-4B \
             --local-dir E:\\AI\\models\\training\\flux2-klein-4b \
             --exclude \"*.jpg\" --exclude \"flux-2-klein-base-4b.safetensors\""
        } else {
            "hf download black-forest-labs/FLUX.2-klein-base-4B \
             --local-dir /srv/models/training/flux2-klein-4b \
             --exclude \"*.jpg\" --exclude \"flux-2-klein-base-4b.safetensors\""
        };
        assert_eq!(hf_download_command(base, store), expected.replace("\n", ""));
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
    fn the_4b_manifest_pins_every_file_the_profile_requires_that_is_a_weight() {
        let base = find_base("flux2-klein-4b").expect("4B base");
        let pinned: Vec<&str> = base.files.iter().map(|f| f.path).collect();
        for required in crate::training::profile::find_for_family("flux2-klein-4b")
            .expect("4B profile")
            .base
            .required_files
        {
            // `model_index.json` is a 422-byte manifest diffusers rewrites on
            // load; the four weight files and the tokenizer are what is pinned.
            if *required == "model_index.json" {
                continue;
            }
            assert!(
                pinned.contains(required),
                "{required} is required but not pinned"
            );
        }
        assert_eq!(base.files.len(), 5, "the 4B snapshot pins five files");
    }

    #[test]
    fn the_quick_hash_target_is_the_smallest_safetensors() {
        let base = find_base("flux2-klein-4b").expect("4B base");
        assert_eq!(
            base.quick_hash_target().map(|f| f.path),
            Some("vae/diffusion_pytorch_model.safetensors")
        );
        assert_eq!(
            find_base("sdxl").and_then(TrainingBase::quick_hash_target),
            None
        );
    }
}
