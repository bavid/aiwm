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
    // 0. A LoRA that an unfinished training run continues from stays. The
    //    row's `init_lora_model_id` is `ON DELETE SET NULL` (migration 0020),
    //    so deleting now would let a later resume re-render its config
    //    without `pretrained_lora_path` and quietly train from scratch.
    if let Some(run) = db
        .training_runs()
        .list_active_for_init_lora(&model.id)
        .await?
        .first()
    {
        return Err(CoreError::Config(format!(
            "\u{201c}{}\u{201d} is the LoRA that \u{201c}{}\u{201d} continues from and that run \
             is still {} — wait until it has finished, or cancel it first",
            model.name,
            run.name,
            run.state.as_str()
        )));
    }

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

    // 2. Remove the canonical file (or, for a Colibri model, the whole model
    //    directory `file_path` points at), then a now-empty per-model parent.
    let file = Path::new(&model.file_path);
    let (file_removed, freed_bytes) = match std::fs::metadata(file) {
        Ok(meta) if meta.is_file() => {
            std::fs::remove_file(file)
                .map_err(|e| CoreError::Config(format!("delete {}: {e}", file.display())))?;
            remove_empty_parent(file);
            (true, meta.len())
        }
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir_all(file)
                .map_err(|e| CoreError::Config(format!("delete {}: {e}", file.display())))?;
            // The recorded size, not a fresh directory walk right before
            // deleting it — `size_bytes` is already the library's source of
            // truth for how much this model weighs (storage report, etc.).
            (true, model.size_bytes.max(0) as u64)
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
    async fn delete_removes_a_directory_based_model_and_frees_its_recorded_size() {
        let db = Database::connect_in_memory().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("qwen36-colibri");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("config.json"), b"{}").unwrap();
        std::fs::write(model_dir.join("shard-0.safetensors"), vec![0u8; 4096]).unwrap();

        let model = db
            .models()
            .insert(NewModel {
                name: "Qwen3.6".into(),
                format: "colibri".into(),
                file_path: model_dir.to_string_lossy().into_owned(),
                size_bytes: 20_000_000_000,
                source: "manual".into(),
                roles: vec!["chat".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();

        let out = delete_model(&db, &model).await.unwrap();
        assert!(out.file_removed);
        assert_eq!(out.freed_bytes, 20_000_000_000);
        assert!(!model_dir.exists(), "the whole model directory is removed");
        assert!(db.models().get(&model.id).await.unwrap().is_none());
    }

    /// A library LoRA plus a run that continues from it, moved through
    /// `states` (an empty list leaves it `preparing`).
    async fn lora_with_continuing_run(
        db: &Database,
        states: &[crate::db::RunState],
    ) -> (Model, crate::db::TrainingRun) {
        let lora = db
            .models()
            .insert(NewModel {
                name: "Anime style v1".into(),
                format: "safetensors".into(),
                file_path: "E:\\AI\\models\\image\\loras\\anime-style-v1.safetensors".into(),
                size_bytes: 128,
                source: "training:run-0".into(),
                roles: vec!["lora".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        let run = db
            .training_runs()
            .create(crate::db::NewTrainingRun {
                name: "Anime style v2".into(),
                profile_family: "flux2-klein-4b".into(),
                target_model_id: None,
                dataset_id: None,
                data_kind: crate::db::DatasetMode::Frames,
                trigger_word: "ghibli_xy".into(),
                preset: crate::db::Preset::Fast,
                hyperparams_json: "{}".into(),
                sample_prompts_json: "[]".into(),
                work_dir: "E:\\AI\\data\\training\\run-1".into(),
                init_lora_model_id: Some(lora.id.clone()),
                image_count: Some(3),
            })
            .await
            .unwrap();
        for next in states {
            db.training_runs().set_state(&run.id, *next).await.unwrap();
        }
        (lora, run)
    }

    #[tokio::test]
    async fn delete_refuses_the_source_lora_of_a_paused_continue_run() {
        // Migration 0020 nulls `init_lora_model_id` on delete; a resume would
        // then render no `pretrained_lora_path` and quietly train from
        // scratch. So while the run can still be resumed, its source stays.
        use crate::db::RunState;
        let db = Database::connect_in_memory().await.unwrap();
        let (lora, run) =
            lora_with_continuing_run(&db, &[RunState::Running, RunState::Paused]).await;

        let err = delete_model(&db, &lora)
            .await
            .expect_err("the source of a paused run must not be deletable");

        assert!(matches!(err, CoreError::Config(_)), "{err}");
        let msg = err.to_string();
        assert!(msg.contains("Anime style v2"), "names the run: {msg}");
        assert!(msg.contains("paused"), "names its state: {msg}");
        assert!(
            db.models().get(&lora.id).await.unwrap().is_some(),
            "the row must still be there"
        );
        assert_eq!(
            db.training_runs()
                .get(&run.id)
                .await
                .unwrap()
                .unwrap()
                .init_lora_model_id
                .as_deref(),
            Some(lora.id.as_str()),
            "the link must be intact"
        );
    }

    #[tokio::test]
    async fn delete_allows_the_source_lora_once_its_continue_run_has_settled() {
        use crate::db::RunState;
        let db = Database::connect_in_memory().await.unwrap();
        let (lora, run) = lora_with_continuing_run(
            &db,
            &[RunState::Running, RunState::Finishing, RunState::Completed],
        )
        .await;

        let out = delete_model(&db, &lora)
            .await
            .expect("a settled run holds nothing");

        assert_eq!(out.id, lora.id);
        assert!(db.models().get(&lora.id).await.unwrap().is_none());
        assert_eq!(
            db.training_runs()
                .get(&run.id)
                .await
                .unwrap()
                .unwrap()
                .init_lora_model_id,
            None,
            "the history keeps the run, with the link nulled by the database"
        );
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
