//! Registering a whole downloaded directory as one Model Library entry.
//!
//! Colibri models (Qwen3.6, OLMoE, …) are a multi-file HF repo snapshot, not
//! a single `.gguf`/`.safetensors` file — they don't go through
//! [`super::import::import_model`]'s single-file hash/move/link pipeline,
//! which assumes exactly one weight file to place and link into a runtime.
//! A directory model is registered where it already sits; nothing is moved.

use std::path::Path;

use crate::db::{Database, Model, NewModel};
use crate::{CoreError, Result};

/// Register `dir` (already in its final resting place) as one Model Library
/// entry. `size_bytes` is the recursive sum of every regular file under
/// `dir`. There is no whole-directory hash — `sha256` stays `None`, so
/// duplicate detection just skips these, same as any model that arrived
/// without one.
pub async fn register_directory_model(
    db: &Database,
    dir: &Path,
    name: &str,
    format: &str,
    roles: Vec<String>,
    ram_estimate_mb: Option<i64>,
) -> Result<Model> {
    let meta = std::fs::metadata(dir)
        .map_err(|e| CoreError::Config(format!("cannot read {}: {e}", dir.display())))?;
    if !meta.is_dir() {
        return Err(CoreError::Config(format!(
            "{} is not a directory",
            dir.display()
        )));
    }

    let scan_dir = dir.to_path_buf();
    let size_bytes = tokio::task::spawn_blocking(move || dir_size(&scan_dir))
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("size worker panicked: {e}")))?;

    let new = NewModel {
        name: name.to_string(),
        format: format.to_string(),
        file_path: dir.to_string_lossy().into_owned(),
        size_bytes: size_bytes.min(i64::MAX as u64) as i64,
        source: "manual".into(),
        roles,
        ram_estimate_mb,
        ..NewModel::default()
    };
    db.models().insert(new).await
}

/// Recursive sum of every regular file's size under `dir`. Best-effort: an
/// unreadable entry is skipped rather than failing the whole scan (mirrors
/// `dir_file_bytes` in `api::handlers`, extended to recurse).
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else if meta.is_file() {
            total += meta.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[tokio::test]
    async fn registers_the_directory_with_its_recursive_size() {
        let db = Database::connect_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let model_dir = tmp.path().join("qwen36-colibri");
        std::fs::create_dir_all(model_dir.join("shards")).unwrap();
        std::fs::write(model_dir.join("config.json"), b"{}").unwrap(); // 2 bytes
        std::fs::write(
            model_dir.join("shards").join("s0.safetensors"),
            vec![0u8; 1000],
        )
        .unwrap();
        std::fs::write(
            model_dir.join("shards").join("s1.safetensors"),
            vec![0u8; 2000],
        )
        .unwrap();

        let model = register_directory_model(
            &db,
            &model_dir,
            "Qwen3.6-35B-A3B",
            "colibri",
            vec!["chat".into()],
            Some(24_576),
        )
        .await
        .unwrap();

        assert_eq!(model.name, "Qwen3.6-35B-A3B");
        assert_eq!(model.format, "colibri");
        assert_eq!(model.file_path, model_dir.to_string_lossy());
        assert_eq!(model.size_bytes, 3002);
        assert_eq!(model.roles, ["chat"]);
        assert_eq!(model.sha256, None);
        assert_eq!(model.ram_estimate_mb, Some(24_576));

        // Actually persisted, not just returned.
        let fetched = db.models().get(&model.id).await.unwrap().unwrap();
        assert_eq!(fetched.size_bytes, 3002);
    }

    #[tokio::test]
    async fn refuses_a_path_that_is_not_a_directory() {
        let db = Database::connect_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();

        let err = register_directory_model(&db, &file, "x", "colibri", vec![], None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a directory"), "got: {err}");
    }

    #[tokio::test]
    async fn refuses_a_missing_directory() {
        let db = Database::connect_in_memory().await.unwrap();
        let err = register_directory_model(
            &db,
            Path::new("Z:\\definitely\\missing"),
            "x",
            "colibri",
            vec![],
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("cannot read"), "got: {err}");
    }
}
