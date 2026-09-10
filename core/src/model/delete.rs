//! Remove a model from the library: tear down its runtime links, delete the
//! canonical file, drop the DB rows (Phase 6.8). Always a confirmed user action
//! — see [`crate::cleanup`] for the reports that surface deletion candidates.

use std::path::Path;

use crate::db::{Database, Model};
use crate::link::{dematerialize, LinkStrategy};
use crate::{CoreError, Result};

/// What a delete actually did (for the confirmation message).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeleteOutcome {
    pub id: String,
    pub name: String,
    /// The canonical file was on disk and got removed.
    pub file_removed: bool,
    /// Bytes freed (`0` when the file was already gone).
    pub freed_bytes: u64,
}

/// Delete `model`. Link teardown and file removal are best-effort (a stale link
/// or a missing file must not block the DB cleanup); a real I/O error removing a
/// present file *is* surfaced.
pub async fn delete_model(db: &Database, model: &Model) -> Result<DeleteOutcome> {
    // 1. Tear down runtime links. Passthrough / ExtraPath are no-ops; junctions
    //    and copies leave something on disk.
    for link in db.models().links(&model.id).await.unwrap_or_default() {
        if let Some(strategy) = LinkStrategy::parse(&link.strategy) {
            if let Err(e) = dematerialize(Path::new(&link.link_path), strategy) {
                tracing::warn!(model = %model.id, link = %link.link_path, error = %e,
                    "could not remove a runtime link during delete");
            }
        }
    }

    // 2. Remove the canonical file, then a now-empty per-model directory.
    let file = Path::new(&model.file_path);
    let (file_removed, freed_bytes) = match std::fs::metadata(file) {
        Ok(meta) if meta.is_file() => {
            std::fs::remove_file(file)
                .map_err(|e| CoreError::Config(format!("delete {}: {e}", file.display())))?;
            remove_empty_parent(file);
            (true, meta.len())
        }
        _ => (false, 0),
    };

    // 3. Drop the row — `model_roles`, `model_links`, `benchmarks` cascade.
    db.models().delete(&model.id).await?;

    Ok(DeleteOutcome {
        id: model.id.clone(),
        name: model.name.clone(),
        file_removed,
        freed_bytes,
    })
}

/// Best-effort: if the file's parent directory is now empty, remove it (LLM
/// models get a per-slug folder; image / video folders are shared and stay).
fn remove_empty_parent(file: &Path) {
    let Some(parent) = file.parent() else { return };
    let empty = std::fs::read_dir(parent)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false);
    if empty {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewModel};

    #[tokio::test]
    async fn delete_removes_the_file_the_folder_and_the_row() {
        let db = Database::connect_in_memory().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("llm").join("qwen");
        std::fs::create_dir_all(&model_dir).unwrap();
        let file = model_dir.join("qwen.gguf");
        std::fs::write(&file, vec![0u8; 4096]).unwrap();

        let model = db
            .models()
            .insert(NewModel {
                name: "Qwen".into(),
                format: "gguf".into(),
                file_path: file.to_string_lossy().into_owned(),
                size_bytes: 4096,
                source: "manual".into(),
                roles: vec!["chat".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        db.models()
            .link_runtime(&model.id, "llamacpp", "passthrough", &model.file_path)
            .await
            .unwrap();

        let out = delete_model(&db, &model).await.unwrap();
        assert!(out.file_removed);
        assert_eq!(out.freed_bytes, 4096);
        assert!(!file.exists());
        assert!(!model_dir.exists(), "empty per-model folder is removed");
        assert!(db.models().get(&model.id).await.unwrap().is_none());
        assert!(db.models().links(&model.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_tolerates_an_already_missing_file() {
        let db = Database::connect_in_memory().await.unwrap();
        let model = db
            .models()
            .insert(NewModel {
                name: "Ghost".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\ghost\\ghost.gguf".into(),
                size_bytes: 4096,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();

        let out = delete_model(&db, &model).await.unwrap();
        assert!(!out.file_removed);
        assert_eq!(out.freed_bytes, 0);
        assert!(db.models().get(&model.id).await.unwrap().is_none());
    }
}
