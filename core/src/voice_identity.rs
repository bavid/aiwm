//! Create/delete a named Dia voice-cloning identity: a reference clip plus
//! its own transcript, set up once under a name (e.g. "Old Man Gareth") and
//! reused across many narration calls — see the Voice tab's "save a voice"
//! flow. Combines the DB row ([`crate::db::VoiceIdentityRepo`]) with the one
//! piece of filesystem work a repo shouldn't own: copying the user's picked
//! file into AIWM's own data dir, so the stored path never depends on a
//! location outside AIWM's control (same principle as `crate::model::import`).

use std::path::{Path, PathBuf};

use crate::db::{Database, VoiceIdentity};
use crate::{CoreError, Result};

/// Body for [`create_voice_identity`].
#[derive(Debug, Clone)]
pub struct CreateVoiceIdentity {
    pub name: String,
    /// A path on this machine, resolved by the caller (a native file picker
    /// in the UI) — copied into `voice_identities_dir`, then never read from
    /// again at this original location.
    pub source_audio_path: PathBuf,
    pub reference_transcript: String,
}

/// Copies `req.source_audio_path` into a fresh subdirectory of
/// `voice_identities_dir` (keeping its original file name), then registers
/// the row. The source file is left untouched — this is a copy, not an
/// import-and-move, since a reference clip is small and the user may want
/// to reuse the original file elsewhere.
pub async fn create_voice_identity(
    db: &Database,
    voice_identities_dir: &Path,
    req: CreateVoiceIdentity,
) -> Result<VoiceIdentity> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(CoreError::Config(
            "voice identity name must not be empty".into(),
        ));
    }
    let transcript = req.reference_transcript.trim();
    if transcript.is_empty() {
        return Err(CoreError::Config(
            "reference transcript must not be empty".into(),
        ));
    }

    let source = &req.source_audio_path;
    let meta = std::fs::metadata(source)
        .map_err(|e| CoreError::Config(format!("cannot read {}: {e}", source.display())))?;
    if !meta.is_file() {
        return Err(CoreError::Config(format!(
            "{} is not a file",
            source.display()
        )));
    }

    let dest = copy_into_store(voice_identities_dir, source)?;
    db.voice_identities()
        .create(name, &dest.to_string_lossy(), transcript)
        .await
}

/// A fresh `<voice_identities_dir>/<uuid>/<original file name>` — the
/// per-identity subdirectory (rather than a flat, hash-suffixed file like
/// model import uses) keeps the reference clip's *original* file name
/// intact, which is occasionally useful when the user goes looking for it
/// (e.g. to reuse the same clip for a different tool).
fn copy_into_store(voice_identities_dir: &Path, source: &Path) -> Result<PathBuf> {
    let dir = voice_identities_dir.join(uuid::Uuid::now_v7().to_string());
    std::fs::create_dir_all(&dir)
        .map_err(|e| CoreError::Config(format!("create {}: {e}", dir.display())))?;
    let filename = source
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_else(|| std::ffi::OsString::from("reference-audio"));
    let dest = dir.join(filename);
    std::fs::copy(source, &dest)
        .map_err(|e| CoreError::Config(format!("copy reference clip: {e}")))?;
    Ok(dest)
}

/// Deletes the identity's DB row and best-effort removes its reference clip
/// (and the per-identity directory it lived in) — a missing file must not
/// block the DB cleanup, mirroring [`crate::model::delete_model`]'s
/// best-effort file removal.
pub async fn delete_voice_identity(db: &Database, identity: &VoiceIdentity) -> Result<()> {
    let file = Path::new(&identity.reference_audio_path);
    if file.is_file() {
        if let Err(e) = std::fs::remove_file(file) {
            tracing::warn!(
                id = %identity.id, path = %identity.reference_audio_path, error = %e,
                "could not remove a voice identity's reference clip during delete"
            );
        } else if let Some(dir) = file.parent() {
            let _ = std::fs::remove_dir(dir); // only succeeds if now empty
        }
    }
    db.voice_identities().delete(&identity.id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str, source: &Path, transcript: &str) -> CreateVoiceIdentity {
        CreateVoiceIdentity {
            name: name.to_string(),
            source_audio_path: source.to_path_buf(),
            reference_transcript: transcript.to_string(),
        }
    }

    #[tokio::test]
    async fn create_copies_the_file_and_registers_the_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("voice-identities");
        let db = Database::connect_in_memory().await.unwrap();
        let source = tmp.path().join("gareth.wav");
        std::fs::write(&source, b"fake-wav-bytes").unwrap();

        let out = create_voice_identity(
            &db,
            &store,
            req("Old Man Gareth", &source, "a scratchy, smokey voice"),
        )
        .await
        .unwrap();

        assert_eq!(out.name, "Old Man Gareth");
        assert_eq!(out.reference_transcript, "a scratchy, smokey voice");
        assert!(source.exists(), "the original file must be left in place");
        assert!(Path::new(&out.reference_audio_path).is_file());
        assert!(out
            .reference_audio_path
            .replace('\\', "/")
            .contains("/voice-identities/"));
        assert!(out.reference_audio_path.ends_with("gareth.wav"));
        assert_eq!(
            std::fs::read(&out.reference_audio_path).unwrap(),
            b"fake-wav-bytes"
        );

        let listed = db.voice_identities().list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, out.id);
    }

    #[tokio::test]
    async fn two_identities_with_the_same_original_filename_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("voice-identities");
        let db = Database::connect_in_memory().await.unwrap();

        let source_a = tmp.path().join("a").join("ref.wav");
        std::fs::create_dir_all(source_a.parent().unwrap()).unwrap();
        std::fs::write(&source_a, b"aaa").unwrap();
        let source_b = tmp.path().join("b").join("ref.wav");
        std::fs::create_dir_all(source_b.parent().unwrap()).unwrap();
        std::fs::write(&source_b, b"bbb").unwrap();

        let a = create_voice_identity(&db, &store, req("A", &source_a, "a"))
            .await
            .unwrap();
        let b = create_voice_identity(&db, &store, req("B", &source_b, "b"))
            .await
            .unwrap();

        assert_ne!(a.reference_audio_path, b.reference_audio_path);
        assert_eq!(std::fs::read(&a.reference_audio_path).unwrap(), b"aaa");
        assert_eq!(std::fs::read(&b.reference_audio_path).unwrap(), b"bbb");
    }

    #[tokio::test]
    async fn rejects_an_empty_name() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let source = tmp.path().join("ref.wav");
        std::fs::write(&source, b"x").unwrap();

        let err = create_voice_identity(&db, &tmp.path().join("store"), req("  ", &source, "t"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("name"), "{err}");
    }

    #[tokio::test]
    async fn rejects_an_empty_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
        let source = tmp.path().join("ref.wav");
        std::fs::write(&source, b"x").unwrap();

        let err = create_voice_identity(&db, &tmp.path().join("store"), req("Name", &source, "  "))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("transcript"), "{err}");
    }

    #[tokio::test]
    async fn rejects_a_missing_source_file() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();

        let err = create_voice_identity(
            &db,
            &tmp.path().join("store"),
            req("Name", &tmp.path().join("nope.wav"), "t"),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("cannot read"), "{err}");
    }

    #[tokio::test]
    async fn delete_removes_the_row_and_the_copied_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("voice-identities");
        let db = Database::connect_in_memory().await.unwrap();
        let source = tmp.path().join("ref.wav");
        std::fs::write(&source, b"x").unwrap();

        let identity = create_voice_identity(&db, &store, req("Name", &source, "t"))
            .await
            .unwrap();
        let copied_path = PathBuf::from(&identity.reference_audio_path);
        assert!(copied_path.is_file());

        delete_voice_identity(&db, &identity).await.unwrap();

        assert!(db
            .voice_identities()
            .get(&identity.id)
            .await
            .unwrap()
            .is_none());
        assert!(!copied_path.exists());
        assert!(
            !copied_path.parent().unwrap().exists(),
            "the now-empty per-identity directory should be cleaned up too"
        );
    }

    #[tokio::test]
    async fn delete_is_best_effort_when_the_file_is_already_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("voice-identities");
        let db = Database::connect_in_memory().await.unwrap();
        let source = tmp.path().join("ref.wav");
        std::fs::write(&source, b"x").unwrap();

        let identity = create_voice_identity(&db, &store, req("Name", &source, "t"))
            .await
            .unwrap();
        std::fs::remove_file(&identity.reference_audio_path).unwrap();

        delete_voice_identity(&db, &identity).await.unwrap();

        assert!(db
            .voice_identities()
            .get(&identity.id)
            .await
            .unwrap()
            .is_none());
    }
}
