//! Load-time integrity check for the pinned captioner snapshot folders
//! (Florence-2 large, Qwen2.5-VL 7B).
//!
//! Import-time pinning (`model::import`) only guards files that go *through*
//! the importer. The folder the sidecar actually loads — with
//! `trust_remote_code=True` for Florence-2 — can still be reached other ways:
//! any model row can be given the `vision_florence2` role, a folder can be
//! registered with `register_directory_model`, or a file can be dropped next
//! to the imported ones. So right before a captioner loads, the folder it
//! will read is checked against the catalog:
//!
//! * it must be exactly `<store>/<kind.store_subdir()>` after
//!   canonicalisation (no junction/symlink redirecting it elsewhere);
//! * every entry must be a regular file named after a catalog file of that
//!   kind, with the catalog's size and SHA-256 — no extra file, no
//!   subfolder (`__pycache__` included);
//! * every catalog file of that kind must be present.
//!
//! Code-bearing files (`.py`, `.json`) are re-hashed on every check. The
//! multi-GB weights are re-hashed only when their (size, mtime) differs from
//! the last successful verification in this process — an in-memory cache,
//! so a restart re-verifies once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use super::{catalog, import::sha256_file, ModelKind};
use crate::{CoreError, Result};

/// One file a verified folder must contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExpectedFile {
    pub file: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
}

fn integrity_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("captioner folder check failed: {msg}"))
}

/// The catalog's pinned files for a directory-shaped captioner kind; `None`
/// for any kind this check does not cover.
fn expected_files(kind: ModelKind) -> Option<Vec<ExpectedFile>> {
    if !matches!(kind, ModelKind::Florence2Engine | ModelKind::QwenVlEngine) {
        return None;
    }
    Some(
        catalog::KNOWN_MODELS
            .iter()
            .filter(|m| m.kind == kind.as_str())
            .map(|m| ExpectedFile {
                file: m.file,
                sha256: m.sha256,
                size_bytes: m.size_bytes,
            })
            .collect(),
    )
}

/// Verify the pinned snapshot folder of `kind` under `store_root` and return
/// its store path (proved to canonicalise to itself, not redirected) — the
/// only directory the sidecar may be pointed at.
/// Blocking (hashes files); call from `spawn_blocking` or use
/// [`verify_captioner_dir_async`].
pub fn verify_captioner_dir(store_root: &Path, kind: ModelKind) -> Result<PathBuf> {
    let expected = expected_files(kind).ok_or_else(|| {
        integrity_err(format!("{} is not a pinned captioner kind", kind.as_str()))
    })?;
    verify_dir(store_root, kind.store_subdir(), &expected)
}

/// [`verify_captioner_dir`] off the async runtime.
pub async fn verify_captioner_dir_async(store_root: &Path, kind: ModelKind) -> Result<PathBuf> {
    let root = store_root.to_path_buf();
    tokio::task::spawn_blocking(move || verify_captioner_dir(&root, kind))
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("integrity worker panicked: {e}")))?
}

/// (canonical path, size, mtime) of a large file whose hash matched in this
/// process -- lets repeat checks skip re-hashing multi-GB weights.
type VerifiedKey = (PathBuf, u64, SystemTime);

fn verified_cache() -> &'static Mutex<HashMap<VerifiedKey, &'static str>> {
    static CACHE: OnceLock<Mutex<HashMap<VerifiedKey, &'static str>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Files that can steer what code runs are re-hashed on every check.
fn always_rehash(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    lower.ends_with(".py") || lower.ends_with(".json")
}

fn verify_dir(store_root: &Path, subdir: &str, expected: &[ExpectedFile]) -> Result<PathBuf> {
    let dir = canonical_store_dir(store_root, subdir)?;
    let mut seen: Vec<&'static str> = Vec::with_capacity(expected.len());

    let entries = std::fs::read_dir(&dir)
        .map_err(|e| integrity_err(format!("cannot list {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| integrity_err(format!("{}: {e}", dir.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(entry.path())
            .map_err(|e| integrity_err(format!("{name}: {e}")))?;
        if !meta.file_type().is_file() {
            return Err(integrity_err(format!(
                "unexpected folder or link \u{201c}{name}\u{201d} in {} — only the pinned \
                 catalog files may be there; remove it",
                dir.display()
            )));
        }
        let want = expected.iter().find(|x| x.file == name).ok_or_else(|| {
            integrity_err(format!(
                "unexpected file \u{201c}{name}\u{201d} in {} — not one of the pinned catalog \
                 files; remove it",
                dir.display()
            ))
        })?;
        verify_file(&entry.path(), want, &meta)?;
        seen.push(want.file);
    }

    if let Some(missing) = expected.iter().find(|x| !seen.contains(&x.file)) {
        return Err(integrity_err(format!(
            "{} is missing from {} — reinstall the stack",
            missing.file,
            dir.display()
        )));
    }
    // The store path, not the `\\?\` verbatim canonical form (which
    // Python tooling handles poorly) -- `canonical_store_dir` proved they are
    // the same directory.
    Ok(store_root.join(subdir))
}

/// `<store>/<subdir>`, canonicalised, and refused when a junction/symlink
/// makes it resolve anywhere other than inside the canonical store root.
fn canonical_store_dir(store_root: &Path, subdir: &str) -> Result<PathBuf> {
    let expected = store_root.join(subdir);
    let dir = expected.canonicalize().map_err(|_| {
        integrity_err(format!(
            "not installed — {} does not exist (install the stack from the Models tab)",
            expected.display()
        ))
    })?;
    let root = store_root
        .canonicalize()
        .map_err(|e| integrity_err(format!("store {}: {e}", store_root.display())))?;
    if dir != root.join(subdir) {
        return Err(integrity_err(format!(
            "{} resolves outside the model store (to {}) — only the store's own folder is \
             loaded",
            expected.display(),
            dir.display()
        )));
    }
    Ok(dir)
}

fn verify_file(path: &Path, want: &ExpectedFile, meta: &std::fs::Metadata) -> Result<()> {
    if meta.len() != want.size_bytes {
        return Err(integrity_err(format!(
            "{} has size {} B, the pinned file is {} B — reinstall the stack",
            want.file,
            meta.len(),
            want.size_bytes
        )));
    }
    let key = meta
        .modified()
        .ok()
        .map(|mtime| (path.to_path_buf(), meta.len(), mtime));
    if !always_rehash(want.file) {
        let cache = verified_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if key
            .as_ref()
            .and_then(|k| cache.get(k))
            .is_some_and(|sha| *sha == want.sha256)
        {
            return Ok(());
        }
    }
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(want.sha256) {
        return Err(integrity_err(format!(
            "{} does not match its pinned SHA-256 (got {actual}) — reinstall the stack",
            want.file
        )));
    }
    if let (Some(k), false) = (key, always_rehash(want.file)) {
        verified_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(k, want.sha256);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBDIR: &str = "vision/florence2-large";
    // sha256("{}") and sha256("weights!") -- computed, not guessed: the
    // `hash_of` test below pins them against `sha256_file`.
    const CONFIG_BODY: &[u8] = b"{}";
    const CONFIG_SHA: &str = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
    const WEIGHTS_BODY: &[u8] = b"weights!";
    const WEIGHTS_SHA: &str = "fa47a1dc2eaad8414f129b768e1bc3ab746de3ee7d73d82c9356a9ca2ad772f4";

    fn expected() -> Vec<ExpectedFile> {
        vec![
            ExpectedFile {
                file: "config.json",
                sha256: CONFIG_SHA,
                size_bytes: CONFIG_BODY.len() as u64,
            },
            ExpectedFile {
                file: "model.safetensors",
                sha256: WEIGHTS_SHA,
                size_bytes: WEIGHTS_BODY.len() as u64,
            },
        ]
    }

    /// A store with a complete, correct folder.
    fn good_store() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), CONFIG_BODY).unwrap();
        std::fs::write(dir.join("model.safetensors"), WEIGHTS_BODY).unwrap();
        let root = tmp.path().to_path_buf();
        (tmp, root)
    }

    fn err_of(root: &Path) -> String {
        verify_dir(root, SUBDIR, &expected())
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn hash_of_the_fixture_bodies_matches_the_constants() {
        let tmp = tempfile::tempdir().unwrap();
        for (body, sha) in [(CONFIG_BODY, CONFIG_SHA), (WEIGHTS_BODY, WEIGHTS_SHA)] {
            let p = tmp.path().join("f");
            std::fs::write(&p, body).unwrap();
            assert_eq!(sha256_file(&p).unwrap(), sha);
        }
    }

    #[test]
    fn a_complete_pinned_folder_verifies_to_its_store_path() {
        let (_tmp, root) = good_store();
        let got = verify_dir(&root, SUBDIR, &expected()).unwrap();
        assert_eq!(got, root.join(SUBDIR));
        // A second check (cache warm for the weights) still passes.
        assert!(verify_dir(&root, SUBDIR, &expected()).is_ok());
    }

    #[test]
    fn a_missing_folder_is_reported_as_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(err_of(tmp.path()).contains("not installed"));
    }

    #[test]
    fn an_extra_file_is_refused_by_name() {
        let (_tmp, root) = good_store();
        std::fs::write(root.join(SUBDIR).join("evil.py"), b"import os").unwrap();
        let e = err_of(&root);
        assert!(e.contains("evil.py"), "{e}");
    }

    #[test]
    fn a_subfolder_is_refused_even_pycache() {
        let (_tmp, root) = good_store();
        std::fs::create_dir(root.join(SUBDIR).join("__pycache__")).unwrap();
        let e = err_of(&root);
        assert!(e.contains("__pycache__"), "{e}");
    }

    #[test]
    fn a_missing_pinned_file_is_named() {
        let (_tmp, root) = good_store();
        std::fs::remove_file(root.join(SUBDIR).join("config.json")).unwrap();
        let e = err_of(&root);
        assert!(e.contains("config.json") && e.contains("missing"), "{e}");
    }

    #[test]
    fn a_tampered_config_with_the_same_size_is_refused() {
        let (_tmp, root) = good_store();
        std::fs::write(root.join(SUBDIR).join("config.json"), b"[]").unwrap();
        let e = err_of(&root);
        assert!(e.contains("config.json"), "{e}");
    }

    #[test]
    fn tampered_weights_are_caught_after_a_successful_check() {
        let (_tmp, root) = good_store();
        verify_dir(&root, SUBDIR, &expected()).unwrap();
        let weights = root.join(SUBDIR).join("model.safetensors");
        // Same size, different bytes; the rewrite moves the mtime, so the
        // cached verification no longer applies.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&weights, b"WEIGHTS!").unwrap();
        let e = err_of(&root);
        assert!(e.contains("model.safetensors"), "{e}");
    }

    #[test]
    fn a_wrong_size_is_refused_without_hashing() {
        let (_tmp, root) = good_store();
        std::fs::write(root.join(SUBDIR).join("model.safetensors"), b"short").unwrap();
        let e = err_of(&root);
        assert!(e.contains("model.safetensors") && e.contains("size"), "{e}");
    }

    /// A folder that lives elsewhere and is only *linked* into the store is
    /// not the store folder: its canonical path differs.
    #[cfg(windows)]
    #[test]
    fn a_folder_redirected_out_of_the_store_is_refused() {
        let (_tmp, root) = good_store();
        let outside = tempfile::tempdir().unwrap();
        let store2 = outside.path().join("store");
        std::fs::create_dir_all(store2.join("vision")).unwrap();
        // A directory junction (unlike a symlink) needs no admin rights.
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            // `mklink` needs backslash-only paths.
            .arg(store2.join("vision").join("florence2-large"))
            .arg(root.join("vision").join("florence2-large"))
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
        let e = err_of(&store2);
        assert!(e.contains("outside"), "{e}");
    }

    #[test]
    fn only_the_two_pinned_captioner_kinds_are_checkable() {
        let tmp = tempfile::tempdir().unwrap();
        let e = verify_captioner_dir(tmp.path(), ModelKind::WdTagger)
            .unwrap_err()
            .to_string();
        assert!(e.contains("wd_tagger"), "{e}");
        let florence = expected_files(ModelKind::Florence2Engine).unwrap();
        assert_eq!(florence.len(), 10);
        assert_eq!(expected_files(ModelKind::QwenVlEngine).unwrap().len(), 14);
    }

    #[test]
    fn the_catalog_folder_check_rejects_an_empty_store() {
        let tmp = tempfile::tempdir().unwrap();
        let e = verify_captioner_dir(tmp.path(), ModelKind::Florence2Engine)
            .unwrap_err()
            .to_string();
        assert!(e.contains("not installed"), "{e}");
    }
}
