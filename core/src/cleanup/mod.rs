//! `core::cleanup` — a storage overview and the "safe to delete" reports
//! (Phase 6.8): what the model store holds, how much room is left on its
//! volume, exact-duplicate weights, and models that have gone unused.
//!
//! Everything here only *reports*. Deleting a model is [`crate::model::delete_model`],
//! always a confirmed user action.

pub mod apply;
pub mod locations;
pub mod outputs;
pub mod scan;

pub use apply::{ApplyRequest, ApplyResult, EntryResult, Selection};
pub(crate) use locations::is_reparse_point;
pub use locations::StorageLocation;
pub use outputs::{RetentionPolicy, SweepResult};
pub use scan::{CleanupEntry, CleanupGroup, CleanupReport, ProtectedNote};

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

use crate::db::Model;

/// Default "not used in a while" threshold for the unused report.
pub const DEFAULT_STALE_DAYS: i64 = 45;
/// Free space kept as a safety margin when gating a download (bytes).
pub const DOWNLOAD_FREE_MARGIN_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The whole picture for the Storage panel.
#[derive(Debug, Clone, Serialize)]
pub struct StorageReport {
    /// Sum of the model file sizes the library knows about.
    pub store_bytes: u64,
    /// Free / total bytes on the volume the store lives on. `None` when the
    /// volume can't be resolved (no matching mount point).
    pub volume_free_bytes: Option<u64>,
    pub volume_total_bytes: Option<u64>,
    pub by_kind: Vec<KindUsage>,
    pub models: Vec<ModelDisk>,
    /// SHA-256 groups with more than one member — the same weights twice.
    pub duplicates: Vec<DuplicateGroup>,
    /// Model ids never used, or not used within `stale_days`.
    pub unused: Vec<String>,
    pub stale_days: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KindUsage {
    /// `llm` / `image` / `video` / `other` — the top store folder.
    pub kind: String,
    pub bytes: u64,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDisk {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: u64,
    pub last_used_at: Option<String>,
    pub use_count: i64,
    pub roles: Vec<String>,
    /// The canonical file is actually on disk.
    pub file_present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    pub sha256: String,
    pub member_ids: Vec<String>,
    /// What deleting all but one would free.
    pub wasted_bytes: u64,
}

/// The top store folder a model's file sits in (`llm` / `image` / `video`),
/// else `"other"` — the file is outside the store.
pub fn kind_of(model: &Model, store_root: &Path) -> String {
    let file = Path::new(&model.file_path);
    let Ok(rel) = file.strip_prefix(store_root) else {
        return "other".to_string();
    };
    rel.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "other".to_string())
}

/// SHA-256 groups with more than one member, newest-first per group.
pub fn duplicates(models: &[Model]) -> Vec<DuplicateGroup> {
    let mut by_hash: BTreeMap<&str, Vec<&Model>> = BTreeMap::new();
    for m in models {
        if let Some(h) = m.sha256.as_deref().filter(|h| !h.is_empty()) {
            by_hash.entry(h).or_default().push(m);
        }
    }
    let mut out: Vec<DuplicateGroup> = by_hash
        .into_iter()
        .filter(|(_, ms)| ms.len() > 1)
        .map(|(hash, mut ms)| {
            ms.sort_by_key(|m| Reverse(m.imported_at.clone()));
            let unit = ms
                .iter()
                .map(|m| m.size_bytes.max(0) as u64)
                .max()
                .unwrap_or(0);
            DuplicateGroup {
                sha256: hash.to_string(),
                member_ids: ms.iter().map(|m| m.id.clone()).collect(),
                wasted_bytes: unit.saturating_mul((ms.len() - 1) as u64),
            }
        })
        .collect();
    out.sort_by_key(|g| Reverse(g.wasted_bytes));
    out
}

/// Ids of models never used, or whose `last_used_at` is before `cutoff_iso`
/// (an RFC 3339 string — timestamps sort lexicographically).
pub fn unused_ids(models: &[Model], cutoff_iso: &str) -> Vec<String> {
    models
        .iter()
        .filter(|m| match &m.last_used_at {
            None => true,
            Some(last) => last.as_str() < cutoff_iso,
        })
        .map(|m| m.id.clone())
        .collect()
}

/// `(free, total)` bytes on the volume a path lives on — the shape of
/// [`volume_free`], behind a function pointer so a preflight disk check
/// (training run, dataset prep) can be driven from a test.
pub type FreeSpaceProbe = fn(&Path) -> Option<(u64, u64)>;

/// The volume a path lives on, for disk messages: `E:` on Windows, the
/// whole path anywhere it has no drive prefix.
pub fn volume_label(path: &Path) -> String {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            prefix.as_os_str().to_string_lossy().into_owned()
        }
        _ => path.display().to_string(),
    }
}

/// `(free, total)` bytes on the volume that `path` lives on — the disk whose
/// mount point is the longest prefix of `path`'s resolved form (links and
/// junctions followed, so a junction into another drive reports that drive).
///
/// On Windows `canonicalize` returns a verbatim `\\?\C:\...` path, which
/// never `starts_with` sysinfo's `C:\` mount point; the prefix is dropped
/// and both sides are compared case-insensitively
/// ([`volume_comparable`]). `None` only when no mount point matches at all
/// (network paths, mounted folders sysinfo does not list).
pub fn volume_free(path: &Path) -> Option<(u64, u64)> {
    let resolved = match path.canonicalize() {
        Ok(canonical) => strip_verbatim(&canonical),
        Err(_) => std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
    };
    let target = volume_comparable(&resolved);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| target.starts_with(volume_comparable(d.mount_point())))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.available_space(), d.total_space()))
}

/// `path` without a Windows verbatim prefix: `\\?\E:\x` → `E:\x`,
/// `\\?\UNC\server\share\x` → `\\server\share\x`. Any other path (a plain
/// drive path, a `\\?\Volume{…}` one, anything on another OS) is returned
/// as is.
fn strip_verbatim(path: &Path) -> std::path::PathBuf {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let plain: std::ffi::OsString = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => format!("{}:", char::from(letter)).into(),
        Prefix::VerbatimUNC(server, share) => {
            let mut s = std::ffi::OsString::from(r"\\");
            s.push(server);
            s.push(r"\");
            s.push(share);
            s
        }
        _ => return path.to_path_buf(),
    };
    let mut out = std::path::PathBuf::from(plain);
    for c in components {
        match c {
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR_STR),
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A path in the form mount points are compared in: verbatim prefix
/// dropped and, on Windows, case-folded. `Path::starts_with` is
/// component-wise, so `E:` and `E:\` both prefix `E:\x`.
fn volume_comparable(path: &Path) -> std::path::PathBuf {
    let plain = strip_verbatim(path);
    if cfg!(windows) {
        std::path::PathBuf::from(plain.to_string_lossy().to_lowercase())
    } else {
        plain
    }
}

/// Assemble the full report. `stale_days` sets the unused cutoff.
pub fn report(models: &[Model], store_root: &Path, stale_days: i64) -> StorageReport {
    let cutoff = (OffsetDateTime::now_utc() - Duration::days(stale_days.max(0)))
        .format(&Rfc3339)
        .unwrap_or_default();

    let mut by_kind: BTreeMap<String, (u64, u32)> = BTreeMap::new();
    let mut store_bytes: u64 = 0;
    let mut disks: Vec<ModelDisk> = Vec::with_capacity(models.len());
    for m in models {
        let size = m.size_bytes.max(0) as u64;
        let kind = kind_of(m, store_root);
        store_bytes = store_bytes.saturating_add(size);
        let e = by_kind.entry(kind.clone()).or_default();
        e.0 = e.0.saturating_add(size);
        e.1 += 1;
        disks.push(ModelDisk {
            id: m.id.clone(),
            name: m.name.clone(),
            kind,
            size_bytes: size,
            last_used_at: m.last_used_at.clone(),
            use_count: m.use_count,
            roles: m.roles.clone(),
            // A Colibri model's `file_path` is a whole downloaded directory,
            // not a single file.
            file_present: {
                let p = Path::new(&m.file_path);
                p.is_file() || p.is_dir()
            },
        });
    }
    disks.sort_by_key(|d| Reverse(d.size_bytes));

    let (free, total) = match volume_free(store_root) {
        Some((f, t)) => (Some(f), Some(t)),
        None => (None, None),
    };

    StorageReport {
        store_bytes,
        volume_free_bytes: free,
        volume_total_bytes: total,
        by_kind: by_kind
            .into_iter()
            .map(|(kind, (bytes, count))| KindUsage { kind, bytes, count })
            .collect(),
        models: disks,
        duplicates: duplicates(models),
        unused: unused_ids(models, &cutoff),
        stale_days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;
    #[cfg(windows)]
    use std::path::PathBuf;

    fn model(over: NewModel) -> Model {
        Model {
            id: over.name.clone(),
            publisher: None,
            name: over.name,
            family: None,
            format: over.format,
            quant: None,
            arch: None,
            param_count: None,
            file_path: over.file_path,
            sha256: over.sha256,
            size_bytes: over.size_bytes,
            ctx_max: None,
            vram_estimate_mb: None,
            ram_estimate_mb: None,
            source: "manual".into(),
            source_revision: None,
            imported_at: "2026-01-01T00:00:00Z".into(),
            last_used_at: None,
            use_count: 0,
            n_layers: None,
            n_embd: None,
            n_heads: None,
            n_kv_heads: None,
            roles: over.roles,
            runtimes: vec![],
        }
    }

    #[test]
    fn kind_is_the_top_store_folder() {
        let store = Path::new("E:\\AI\\models");
        let m = model(NewModel {
            name: "q".into(),
            format: "gguf".into(),
            file_path: "E:\\AI\\models\\llm\\qwen\\q.gguf".into(),
            ..NewModel::default()
        });
        assert_eq!(kind_of(&m, store), "llm");

        let outside = model(NewModel {
            name: "x".into(),
            format: "gguf".into(),
            file_path: "D:\\elsewhere\\x.gguf".into(),
            ..NewModel::default()
        });
        assert_eq!(kind_of(&outside, store), "other");
    }

    #[test]
    fn duplicates_groups_by_sha256_and_totals_the_waste() {
        let mk = |name: &str, hash: Option<&str>, size: i64, imported: &str| {
            let mut m = model(NewModel {
                name: name.into(),
                format: "gguf".into(),
                file_path: format!("E:\\m\\{name}.gguf"),
                sha256: hash.map(str::to_string),
                size_bytes: size,
                ..NewModel::default()
            });
            m.imported_at = imported.into();
            m
        };
        let models = vec![
            mk("a", Some("aaaa"), 1_000, "2026-01-01T00:00:00Z"),
            mk("a-copy", Some("aaaa"), 1_000, "2026-02-01T00:00:00Z"),
            mk("a-again", Some("aaaa"), 1_000, "2026-03-01T00:00:00Z"),
            mk("b", Some("bbbb"), 500, "2026-01-01T00:00:00Z"),
            mk("no-hash", None, 9_999, "2026-01-01T00:00:00Z"),
        ];
        let dups = duplicates(&models);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].sha256, "aaaa");
        assert_eq!(dups[0].member_ids, ["a-again", "a-copy", "a"]); // newest first
        assert_eq!(dups[0].wasted_bytes, 2_000); // two redundant copies
    }

    #[test]
    fn unused_ids_flags_never_used_and_stale() {
        let mut fresh = model(NewModel {
            name: "fresh".into(),
            format: "gguf".into(),
            file_path: "E:\\m\\fresh.gguf".into(),
            ..NewModel::default()
        });
        fresh.last_used_at = Some("2026-09-01T00:00:00Z".into());
        let mut stale = model(NewModel {
            name: "stale".into(),
            format: "gguf".into(),
            file_path: "E:\\m\\stale.gguf".into(),
            ..NewModel::default()
        });
        stale.last_used_at = Some("2026-01-01T00:00:00Z".into());
        let never = model(NewModel {
            name: "never".into(),
            format: "gguf".into(),
            file_path: "E:\\m\\never.gguf".into(),
            ..NewModel::default()
        });

        let got = unused_ids(&[fresh, stale, never], "2026-06-01T00:00:00Z");
        assert_eq!(got, ["stale", "never"]);
    }

    #[test]
    fn report_sums_by_kind_and_orders_models_by_size() {
        let store = Path::new("E:\\AI\\models");
        let mk = |name: &str, kind: &str, size: i64| {
            model(NewModel {
                name: name.into(),
                format: "gguf".into(),
                file_path: format!("E:\\AI\\models\\{kind}\\{name}\\f"),
                size_bytes: size,
                ..NewModel::default()
            })
        };
        let r = report(
            &[
                mk("big", "llm", 8_000),
                mk("small", "llm", 1_000),
                mk("pic", "image", 6_000),
            ],
            store,
            30,
        );
        assert_eq!(r.store_bytes, 15_000);
        assert_eq!(r.models[0].name, "big");
        let llm = r.by_kind.iter().find(|k| k.kind == "llm").unwrap();
        assert_eq!(llm.bytes, 9_000);
        assert_eq!(llm.count, 2);
        assert_eq!(r.unused.len(), 3); // none ever used
    }

    /// The real bug: `canonicalize` yields a verbatim `\\?\C:\...` path on
    /// Windows, which never `starts_with` sysinfo's `C:\` mount point, so
    /// every probe came back `None` (and every free-space gate fail-opened).
    #[cfg(windows)]
    #[test]
    fn volume_free_resolves_the_temp_dirs_volume_on_windows() {
        let temp = std::env::temp_dir();
        let (free, total) =
            volume_free(&temp).expect("the temp dir lives on a mounted local volume");
        eprintln!(
            "volume_free({}) = free {free} / total {total}",
            temp.display()
        );
        assert!(free > 0, "free must be positive, got {free}");
        assert!(total >= free, "total {total} must be >= free {free}");
    }

    /// The prefix a Windows `canonicalize` adds is dropped; plain paths are
    /// left alone (case included — folding is the comparison's job).
    #[cfg(windows)]
    #[test]
    fn strip_verbatim_drops_only_the_verbatim_prefix() {
        let cases = [
            (r"\\?\E:\x", r"E:\x"),
            (r"\\?\UNC\server\share\x", r"\\server\share\x"),
            (r"E:\x", r"E:\x"),
            (r"e:\x", r"e:\x"),
        ];
        for (input, want) in cases {
            assert_eq!(
                strip_verbatim(Path::new(input)),
                PathBuf::from(want),
                "strip_verbatim({input:?})"
            );
        }
    }

    /// Mount points compare case-insensitively on Windows, with or without a
    /// trailing separator.
    #[cfg(windows)]
    #[test]
    fn volume_comparable_folds_case_and_trailing_separators() {
        let target = volume_comparable(Path::new(r"e:\AI\data"));
        assert!(target.starts_with(volume_comparable(Path::new(r"E:\"))));
        assert!(target.starts_with(volume_comparable(Path::new(r"E:"))));
        assert!(!target.starts_with(volume_comparable(Path::new(r"D:\"))));
    }

    #[test]
    fn report_treats_a_directory_based_model_as_present() {
        let tmp = tempfile::tempdir().unwrap();
        let model_dir = tmp.path().join("qwen36");
        std::fs::create_dir_all(&model_dir).unwrap();

        let m = model(NewModel {
            name: "Qwen3.6".into(),
            format: "colibri".into(),
            file_path: model_dir.to_string_lossy().into_owned(),
            size_bytes: 20_000,
            ..NewModel::default()
        });
        let r = report(&[m], tmp.path(), 30);
        assert!(
            r.models[0].file_present,
            "a directory-based model's own directory must count as present"
        );
    }
}
