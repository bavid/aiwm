//! Making a canonical model file reachable from a runtime's own directory
//! layout (ADR-007).
//!
//! - [`Passthrough`](LinkStrategy::Passthrough): the runtime takes an absolute
//!   path — nothing to create (llama.cpp).
//! - [`Junction`](LinkStrategy::Junction): an NTFS directory reparse point from
//!   `<runtime>/…/<slug>` to the canonical `<store>/llm/<slug>` — same volume,
//!   no admin (ComfyUI, LM Studio).
//! - [`Hardlink`](LinkStrategy::Hardlink): a file hardlink; same volume only.
//! - [`Copy`](LinkStrategy::Copy): a real copy — the last resort, and Ollama's
//!   only option.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

/// Windows error code for a cross-volume hardlink / rename.
const ERROR_NOT_SAME_DEVICE: i32 = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStrategy {
    Passthrough,
    Junction,
    Hardlink,
    Copy,
}

impl LinkStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::Junction => "junction",
            Self::Hardlink => "hardlink",
            Self::Copy => "copy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "passthrough" => Self::Passthrough,
            "junction" => Self::Junction,
            "hardlink" => Self::Hardlink,
            "copy" => Self::Copy,
            _ => return None,
        })
    }
}

/// The strategy that makes `_format` reachable to `runtime_id`.
pub fn strategy_for(runtime_id: &str, _format: &str) -> LinkStrategy {
    match runtime_id {
        // llama-server takes `-m <absolute path>`.
        "llamacpp" => LinkStrategy::Passthrough,
        // Ollama's store is content-addressed — it must own its own copy.
        "ollama" => LinkStrategy::Copy,
        // ComfyUI / LM Studio scan a directory tree.
        _ => LinkStrategy::Junction,
    }
}

fn link_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("link: {msg}"))
}

/// Make `canonical_file` reachable at (or through) `dest`, returning the exact
/// path the runtime should hand its loader. Idempotent — an existing correct
/// link is left alone.
///
/// For [`Junction`](LinkStrategy::Junction), `dest` is the *directory* to create;
/// for the others it is the *file* path.
pub fn materialize(canonical_file: &Path, dest: &Path, strategy: LinkStrategy) -> Result<PathBuf> {
    if !canonical_file.is_file() {
        return Err(link_err(format!(
            "canonical file is missing: {}",
            canonical_file.display()
        )));
    }
    match strategy {
        LinkStrategy::Passthrough => Ok(canonical_file.to_path_buf()),

        LinkStrategy::Junction => {
            let src_dir = canonical_file
                .parent()
                .ok_or_else(|| link_err("canonical file has no parent directory"))?;
            let filename = canonical_file
                .file_name()
                .ok_or_else(|| link_err("canonical file has no name"))?;
            create_junction(src_dir, dest)?;
            Ok(dest.join(filename))
        }

        LinkStrategy::Hardlink => {
            ensure_parent(dest)?;
            if !dest.exists() {
                std::fs::hard_link(canonical_file, dest).map_err(|e| {
                    if e.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE)
                        || e.kind() == std::io::ErrorKind::CrossesDevices
                    {
                        link_err(format!(
                            "hardlink needs both paths on one volume ({} vs {}) — use a junction or copy",
                            canonical_file.display(),
                            dest.display()
                        ))
                    } else {
                        link_err(format!("hardlink {}: {e}", dest.display()))
                    }
                })?;
            }
            Ok(dest.to_path_buf())
        }

        LinkStrategy::Copy => {
            ensure_parent(dest)?;
            if !dest.exists() {
                std::fs::copy(canonical_file, dest)
                    .map_err(|e| link_err(format!("copy to {}: {e}", dest.display())))?;
            }
            Ok(dest.to_path_buf())
        }
    }
}

/// Undo a [`materialize`] at `dest`. Never touches the canonical file.
pub fn dematerialize(dest: &Path, strategy: LinkStrategy) -> Result<()> {
    match strategy {
        LinkStrategy::Passthrough => Ok(()),
        LinkStrategy::Junction => remove_junction(dest),
        LinkStrategy::Hardlink | LinkStrategy::Copy => {
            let _ = std::fs::remove_file(dest);
            Ok(())
        }
    }
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| link_err(format!("create {}: {e}", parent.display())))?;
    }
    Ok(())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(windows)]
fn create_junction(target_dir: &Path, link_dir: &Path) -> Result<()> {
    ensure_parent(link_dir)?;
    if junction::exists(link_dir).unwrap_or(false) {
        if junction::get_target(link_dir).is_ok_and(|t| same_dir(&t, target_dir)) {
            return Ok(());
        }
        remove_junction(link_dir)?; // wrong target — replace it
    }
    junction::create(target_dir, link_dir).map_err(|e| {
        link_err(format!(
            "create junction {} -> {}: {e}",
            link_dir.display(),
            target_dir.display()
        ))
    })
}

#[cfg(windows)]
fn remove_junction(link_dir: &Path) -> Result<()> {
    if junction::exists(link_dir).unwrap_or(false) {
        junction::delete(link_dir)
            .map_err(|e| link_err(format!("delete junction {}: {e}", link_dir.display())))?;
    }
    // `junction::delete` clears the reparse point but leaves the empty directory.
    let _ = std::fs::remove_dir(link_dir);
    Ok(())
}

#[cfg(not(windows))]
fn create_junction(target_dir: &Path, link_dir: &Path) -> Result<()> {
    // Non-Windows is dev-only; a symlink stands in for a junction.
    ensure_parent(link_dir)?;
    if link_dir.exists() {
        return Ok(());
    }
    std::os::unix::fs::symlink(target_dir, link_dir)
        .map_err(|e| link_err(format!("symlink {}: {e}", link_dir.display())))
}

#[cfg(not(windows))]
fn remove_junction(link_dir: &Path) -> Result<()> {
    let _ = std::fs::remove_file(link_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn strategy_round_trips_as_string() {
        for s in [
            LinkStrategy::Passthrough,
            LinkStrategy::Junction,
            LinkStrategy::Hardlink,
            LinkStrategy::Copy,
        ] {
            assert_eq!(LinkStrategy::parse(s.as_str()), Some(s));
        }
        assert_eq!(LinkStrategy::parse("bogus"), None);
    }

    #[test]
    fn strategy_for_known_runtimes() {
        assert_eq!(strategy_for("llamacpp", "gguf"), LinkStrategy::Passthrough);
        assert_eq!(strategy_for("ollama", "gguf"), LinkStrategy::Copy);
        assert_eq!(
            strategy_for("comfyui", "safetensors"),
            LinkStrategy::Junction
        );
    }

    #[test]
    fn passthrough_returns_the_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("llm/qwen/model.gguf");
        write(&file, b"GGUF");
        let got = materialize(
            &file,
            &tmp.path().join("ignored"),
            LinkStrategy::Passthrough,
        )
        .unwrap();
        assert_eq!(got, file);
    }

    #[test]
    fn copy_creates_an_independent_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("llm/qwen/model.gguf");
        write(&file, b"GGUF-body");
        let dest = tmp.path().join("runtime/models/qwen.gguf");

        let got = materialize(&file, &dest, LinkStrategy::Copy).unwrap();
        assert_eq!(got, dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"GGUF-body");

        // Idempotent.
        materialize(&file, &dest, LinkStrategy::Copy).unwrap();

        dematerialize(&dest, LinkStrategy::Copy).unwrap();
        assert!(!dest.exists());
        assert!(file.is_file(), "canonical file must survive");
    }

    #[test]
    fn hardlink_shares_content_on_the_same_volume() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("llm/qwen/model.gguf");
        write(&file, b"shared");
        let dest = tmp.path().join("runtime/qwen.gguf");

        let got = materialize(&file, &dest, LinkStrategy::Hardlink).unwrap();
        assert_eq!(got, dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"shared");

        dematerialize(&dest, LinkStrategy::Hardlink).unwrap();
        assert!(!dest.exists());
        assert!(file.is_file());
    }

    #[cfg(windows)]
    #[test]
    fn junction_exposes_the_canonical_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().join("store/llm/qwen/model.gguf");
        write(&canonical, b"through-the-junction");
        let link_dir = tmp.path().join("runtime/models/qwen");

        let served = materialize(&canonical, &link_dir, LinkStrategy::Junction).unwrap();
        assert_eq!(served, link_dir.join("model.gguf"));
        assert_eq!(std::fs::read(&served).unwrap(), b"through-the-junction");

        // Idempotent re-link.
        materialize(&canonical, &link_dir, LinkStrategy::Junction).unwrap();

        dematerialize(&link_dir, LinkStrategy::Junction).unwrap();
        assert!(!link_dir.exists());
        assert!(
            canonical.is_file(),
            "removing the junction must not touch the store"
        );
    }

    #[test]
    fn materialize_rejects_a_missing_canonical_file() {
        let tmp = tempfile::tempdir().unwrap();
        let err = materialize(
            &tmp.path().join("nope.gguf"),
            &tmp.path().join("x.gguf"),
            LinkStrategy::Copy,
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
}
