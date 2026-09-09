//! Export / import a portable backup (Phase 5.5).
//!
//! An export is a small `.zip`: a consistent `aiwm.db` snapshot, `config.toml`,
//! a model manifest (names + SHA-256, **not** the multi-GB model files) and a
//! little metadata. Agent profiles, sessions and transcripts all live in the DB,
//! so nothing else needs bundling.
//!
//! Import can't overwrite an open SQLite file, so it **stages** the db + config
//! under `<data>/.pending-import/`; the next [`apply_pending_import`] on startup
//! swaps them in (keeping the old ones as `*.pre-import`).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::{now_rfc3339, Database};
use crate::paths::AppPaths;
use crate::{CoreError, Result};

const META_ENTRY: &str = "aiwm-export.json";
const DB_ENTRY: &str = "aiwm.db";
const CONFIG_ENTRY: &str = "config.toml";
const MANIFEST_ENTRY: &str = "models.json";
const FORMAT: u32 = 1;
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

fn backup_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("backup: {msg}"))
}
fn io_err(e: std::io::Error) -> CoreError {
    CoreError::Io(e)
}
fn zip_err(e: zip::result::ZipError) -> CoreError {
    backup_err(format!("archive: {e}"))
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportMeta {
    format: u32,
    core_version: String,
    created_at: String,
    model_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelManifestEntry {
    name: String,
    sha256: Option<String>,
    size_bytes: i64,
    /// Basename only — the manifest tells you *which* models to re-import, not
    /// where they were.
    file: String,
    roles: Vec<String>,
}

/// What an import staged, for the UI to relay before the restart.
#[derive(Debug, Clone, Serialize)]
pub struct ImportSummary {
    pub core_version: String,
    pub created_at: String,
    pub model_count: usize,
    /// Models in the archive whose file is not in this machine's store — the
    /// user must re-import these (`"<name> (<file>)"`).
    pub missing_models: Vec<String>,
    pub restart_required: bool,
}

/// Build the export archive in memory.
pub async fn export(paths: &AppPaths, db: &Database) -> Result<Vec<u8>> {
    std::fs::create_dir_all(paths.exports_dir()).map_err(io_err)?;
    let snapshot = paths
        .exports_dir()
        .join(format!(".snapshot-{}.db", now_stamp()));
    let _ = std::fs::remove_file(&snapshot);
    db.snapshot_to(&snapshot).await?;
    let db_bytes = std::fs::read(&snapshot).map_err(io_err)?;
    let _ = std::fs::remove_file(&snapshot);

    let config_bytes = std::fs::read(paths.config_file()).unwrap_or_default();

    let manifest: Vec<ModelManifestEntry> = db
        .models()
        .list()
        .await?
        .into_iter()
        .map(|m| ModelManifestEntry {
            name: m.name,
            sha256: m.sha256,
            size_bytes: m.size_bytes,
            file: Path::new(&m.file_path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            roles: m.roles,
        })
        .collect();

    let meta = ExportMeta {
        format: FORMAT,
        core_version: crate::CORE_VERSION.to_string(),
        created_at: now_rfc3339(),
        model_count: manifest.len(),
    };

    let mut buf = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in [
            (META_ENTRY, json_bytes(&meta)),
            (DB_ENTRY, db_bytes),
            (CONFIG_ENTRY, config_bytes),
            (MANIFEST_ENTRY, json_bytes(&manifest)),
        ] {
            zw.start_file(name, opts).map_err(zip_err)?;
            zw.write_all(&bytes).map_err(io_err)?;
        }
        zw.finish().map_err(zip_err)?;
    }
    Ok(buf)
}

/// Export to `<data>/exports/aiwm-export-<ts>.zip`; returns the path.
pub async fn export_to_file(paths: &AppPaths, db: &Database) -> Result<PathBuf> {
    let bytes = export(paths, db).await?;
    let path = paths
        .exports_dir()
        .join(format!("aiwm-export-{}.zip", now_stamp()));
    std::fs::write(&path, bytes).map_err(io_err)?;
    Ok(path)
}

/// Validate the archive and stage its db + config for the next startup. Does
/// **not** touch the live database.
pub async fn stage_import(
    paths: &AppPaths,
    zip_bytes: &[u8],
    db: &Database,
) -> Result<ImportSummary> {
    let mut zr = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| backup_err(format!("not a valid .zip: {e}")))?;

    let meta: ExportMeta = read_json(&mut zr, META_ENTRY)?;
    if meta.format != FORMAT {
        return Err(backup_err(format!(
            "this build reads export format {FORMAT}, the archive is {}",
            meta.format
        )));
    }
    let manifest: Vec<ModelManifestEntry> = read_json(&mut zr, MANIFEST_ENTRY)?;
    let db_bytes = read_entry(&mut zr, DB_ENTRY)?;
    if !db_bytes.starts_with(SQLITE_MAGIC) {
        return Err(backup_err("the archive's aiwm.db is not a SQLite database"));
    }
    let config_bytes = read_entry(&mut zr, CONFIG_ENTRY).unwrap_or_default();

    let have: Vec<String> = db
        .models()
        .list()
        .await?
        .into_iter()
        .filter_map(|m| m.sha256)
        .collect();
    let missing_models = manifest
        .iter()
        .filter(|e| e.sha256.as_ref().is_none_or(|s| !have.contains(s)))
        .map(|e| format!("{} ({})", e.name, e.file))
        .collect();

    let staging = paths.pending_import_dir();
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(io_err)?;
    std::fs::write(staging.join("aiwm.db"), &db_bytes).map_err(io_err)?;
    if !config_bytes.is_empty() {
        std::fs::write(staging.join("config.toml"), &config_bytes).map_err(io_err)?;
    }

    Ok(ImportSummary {
        core_version: meta.core_version,
        created_at: meta.created_at,
        model_count: meta.model_count,
        missing_models,
        restart_required: true,
    })
}

/// Called by [`crate::app::App::load`] before the database is opened. Swaps a
/// staged import in (old files kept as `*.pre-import`). Returns whether it did.
pub fn apply_pending_import(paths: &AppPaths) -> Result<bool> {
    let staging = paths.pending_import_dir();
    let staged_db = staging.join("aiwm.db");
    if !staged_db.is_file() {
        return Ok(false);
    }

    if paths.db_file().is_file() {
        replace(&paths.db_file(), &sibling(paths, "aiwm.db.pre-import"))?;
        for wal in ["aiwm.db-wal", "aiwm.db-shm"] {
            let _ = std::fs::remove_file(paths.root().join(wal));
        }
    }
    std::fs::rename(&staged_db, paths.db_file()).map_err(io_err)?;

    let staged_config = staging.join("config.toml");
    if staged_config.is_file() {
        if paths.config_file().is_file() {
            replace(
                &paths.config_file(),
                &sibling(paths, "config.toml.pre-import"),
            )?;
        }
        std::fs::rename(&staged_config, paths.config_file()).map_err(io_err)?;
    }

    let _ = std::fs::remove_dir_all(&staging);
    tracing::warn!(
        "applied a staged backup import — the previous db/config are kept as *.pre-import"
    );
    Ok(true)
}

// --- helpers --------------------------------------------------------------

fn sibling(paths: &AppPaths, name: &str) -> PathBuf {
    paths.root().join(name)
}

/// `rename(from, to)`, replacing `to` if it exists.
fn replace(from: &Path, to: &Path) -> Result<()> {
    let _ = std::fs::remove_file(to);
    std::fs::rename(from, to).map_err(io_err)
}

fn now_stamp() -> String {
    now_rfc3339().replace([':', '.', '+'], "-")
}

fn json_bytes<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap_or_default()
}

fn read_entry<R: std::io::Read + std::io::Seek>(
    zr: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let mut e = zr
        .by_name(name)
        .map_err(|_| backup_err(format!("archive has no {name}")))?;
    let mut out = Vec::with_capacity(e.size() as usize);
    e.read_to_end(&mut out).map_err(io_err)?;
    Ok(out)
}

fn read_json<R: std::io::Read + std::io::Seek, T: for<'de> Deserialize<'de>>(
    zr: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<T> {
    let bytes = read_entry(zr, name)?;
    serde_json::from_slice(&bytes).map_err(|e| backup_err(format!("bad {name}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;

    /// An **on-disk** db under `paths` with one model — `export` snapshots it
    /// via `VACUUM INTO`, which is a no-op from a `:memory:` source.
    async fn db_with_a_model(paths: &AppPaths) -> Database {
        let db = Database::connect(&paths.db_file()).await.unwrap();
        db.models()
            .insert(NewModel {
                name: "Qwen2.5 Coder".into(),
                format: "gguf".into(),
                file_path: "E:\\models\\coder.gguf".into(),
                size_bytes: 4096,
                sha256: Some("a".repeat(64)),
                source: "manual".into(),
                roles: vec!["coding".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn export_produces_an_archive_with_every_part() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        paths.ensure().unwrap();
        std::fs::write(paths.config_file(), "offline_mode = true\n").unwrap();
        let db = db_with_a_model(&paths).await;

        let bytes = export(&paths, &db).await.unwrap();
        let mut zr = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let names: Vec<_> = zr.file_names().map(str::to_string).collect();
        assert!(names.contains(&"aiwm.db".to_string()));
        assert!(names.contains(&"config.toml".to_string()));
        assert!(names.contains(&"models.json".to_string()));

        let meta: ExportMeta = read_json(&mut zr, META_ENTRY).unwrap();
        assert_eq!(meta.format, 1);
        assert_eq!(meta.model_count, 1);
        assert!(read_entry(&mut zr, "aiwm.db")
            .unwrap()
            .starts_with(SQLITE_MAGIC));
        assert!(
            String::from_utf8_lossy(&read_entry(&mut zr, "config.toml").unwrap())
                .contains("offline_mode")
        );
    }

    #[tokio::test]
    async fn stage_then_apply_swaps_the_db_and_flags_missing_models() {
        let src = tempfile::tempdir().unwrap();
        let sp = AppPaths::rooted(src.path());
        sp.ensure().unwrap();
        std::fs::write(sp.config_file(), "vram_budget_mb = 12000\n").unwrap();
        let bytes = export(&sp, &db_with_a_model(&sp).await).await.unwrap();

        // A different machine: empty store, its own db + config.
        let dst = tempfile::tempdir().unwrap();
        let dp = AppPaths::rooted(dst.path());
        dp.ensure().unwrap();
        std::fs::write(dp.config_file(), "old = true\n").unwrap();
        std::fs::write(dp.db_file(), b"SQLite format 3\0old-db").unwrap();
        let dst_db = Database::connect_in_memory().await.unwrap();

        let summary = stage_import(&dp, &bytes, &dst_db).await.unwrap();
        assert!(summary.restart_required);
        assert_eq!(summary.missing_models.len(), 1);
        assert!(summary.missing_models[0].contains("Qwen2.5 Coder"));
        // The live files are untouched until the swap.
        assert_eq!(
            std::fs::read(dp.db_file()).unwrap(),
            b"SQLite format 3\0old-db"
        );

        assert!(apply_pending_import(&dp).unwrap());
        assert!(std::fs::read(dp.db_file())
            .unwrap()
            .starts_with(SQLITE_MAGIC));
        assert!(std::fs::read_to_string(dp.config_file())
            .unwrap()
            .contains("vram_budget_mb"));
        assert!(dp.root().join("aiwm.db.pre-import").is_file());
        assert!(!dp.pending_import_dir().exists());
        // A second startup is a no-op.
        assert!(!apply_pending_import(&dp).unwrap());
    }

    #[tokio::test]
    async fn a_junk_archive_is_rejected_and_stages_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        paths.ensure().unwrap();
        let db = Database::connect_in_memory().await.unwrap();

        let err = stage_import(&paths, b"not a zip at all", &db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a valid .zip"), "{err}");
        assert!(!paths.pending_import_dir().exists());
    }
}
