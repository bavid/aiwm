//! Captioner registry (spec 3D): Florence-2 is one entry, not the pipeline.
//! Each captioner is resolved from the model library by role, exactly like
//! `caption::resolve_captioner_dir` does for every entry; `installed` is what the
//! Dataset tab's "Beschreiben mit" dropdown filters on.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::compose::CaptionStyle;
use crate::db::Database;
use crate::model::{ModelKind, WD_TAGGER_ROLE};
use crate::Result;

pub const FLORENCE2_ID: &str = "florence2";
pub const WD_TAGGER_ID: &str = "wd-eva02-tagger-v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Captioner {
    pub id: &'static str,
    pub name: &'static str,
    pub style: CaptionStyle,
    /// Model-library role its files are imported under.
    pub role: &'static str,
    /// VRAM the vision runtime must reserve to run it (0 for a CPU-only
    /// onnxruntime tagger).
    pub vram_mb: u64,
    /// The license its weights ship under — shown next to the entry in the
    /// UI, same reasoning as every `KnownModel::license`.
    pub license: &'static str,
    /// Whether the frame-X-vs-X+N temporal escalation applies on top of it.
    /// A tagger has no sentence to judge for confidence, so it cannot.
    pub supports_escalation: bool,
    /// File names that must all sit in one directory for the captioner to
    /// be usable — every file its loader reads, which is also exactly the
    /// file set of its one-click catalog stack, so a stack still
    /// downloading never reads as installed.
    pub required_files: &'static [&'static str],
}

pub const CAPTIONERS: &[Captioner] = &[
    Captioner {
        id: FLORENCE2_ID,
        name: "Florence-2 (prose)",
        style: CaptionStyle::Prose,
        role: super::caption::FLORENCE2_ROLE,
        vram_mb: super::caption::FLORENCE2_VRAM_FALLBACK_MB,
        license: "MIT",
        supports_escalation: true,
        required_files: super::caption::FLORENCE2_REQUIRED_FILES,
    },
    Captioner {
        id: WD_TAGGER_ID,
        name: "WD EVA02 Tagger v3 (Danbooru tags)",
        style: CaptionStyle::Tags,
        role: WD_TAGGER_ROLE,
        vram_mb: 0, // onnxruntime on the CPU
        license: "Apache-2.0",
        supports_escalation: false,
        required_files: &["model.onnx", "selected_tags.csv"],
    },
];

pub fn find_captioner(id: &str) -> Option<&'static Captioner> {
    CAPTIONERS.iter().find(|c| c.id == id)
}

/// A registry entry plus whether its files are in the library — what the
/// UI lists.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaptionerStatus {
    #[serde(flatten)]
    pub captioner: Captioner,
    /// Whether [`installed_captioner_dir`] found a directory satisfying
    /// this captioner — not merely "some row carries the role" (a lone
    /// `selected_tags.csv` import must not read as an installed tagger).
    /// `false` as well when the files are all there but fail the load-time
    /// integrity check — see `unusable`.
    pub installed: bool,
    /// Why a complete set of files still cannot be used (the pinned snapshot
    /// folder failed [`crate::model::verify_captioner_dir`]: a tampered,
    /// missing or extra file). `None` when installed or simply not there.
    pub unusable: Option<String>,
}

/// The Qwen2.5-VL escalation model's state for the Models tab — it is not a
/// captioner of its own, but it has the same "files there, yet unusable"
/// case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EscalationStatus {
    /// Every required file sits in one directory of the library.
    pub files_present: bool,
    /// Files present and the pinned folder passes the integrity check.
    pub usable: bool,
    /// Why present files are not usable.
    pub reason: Option<String>,
}

/// The directory that satisfies `c`: see [`complete_dir_for_role`].
pub async fn installed_captioner_dir(db: &Database, c: &Captioner) -> Result<Option<PathBuf>> {
    complete_dir_for_role(db, c.role, c.required_files).await
}

/// The first (sorted, for determinism) directory among `role`'s model rows
/// whose sibling file names cover every entry in `required`. `None` when no
/// directory qualifies, including when the role has no rows at all.
pub async fn complete_dir_for_role(
    db: &Database,
    role: &str,
    required: &[&str],
) -> Result<Option<PathBuf>> {
    let rows = db.models().for_role(role).await?;
    let entries: Vec<(&Path, &str)> = rows
        .iter()
        .filter_map(|m| {
            let path = Path::new(&m.file_path);
            let name = path.file_name().and_then(|f| f.to_str())?;
            Some((path.parent()?, name))
        })
        .collect();

    let mut dirs: Vec<&Path> = entries.iter().map(|(dir, _)| *dir).collect();
    dirs.sort();
    dirs.dedup();

    Ok(dirs
        .into_iter()
        .find(|dir| {
            required
                .iter()
                .all(|req| entries.iter().any(|(d, name)| d == dir && name == req))
        })
        .map(Path::to_path_buf))
}

pub async fn captioner_statuses(db: &Database) -> Result<Vec<CaptionerStatus>> {
    let mut out = Vec::with_capacity(CAPTIONERS.len());
    for c in CAPTIONERS {
        let installed = installed_captioner_dir(db, c).await?.is_some();
        out.push(CaptionerStatus {
            captioner: *c,
            installed,
            unusable: None,
        });
    }
    Ok(out)
}

/// The pinned snapshot kind whose files carry `role` — the kinds whose
/// folder the load-time integrity check covers (Florence-2, Qwen2.5-VL).
fn pinned_kind_for_role(role: &str) -> Option<ModelKind> {
    [ModelKind::Florence2Engine, ModelKind::QwenVlEngine]
        .into_iter()
        .find(|k| k.default_role() == Some(role))
}

/// The load-time integrity verdict for a pinned kind's store folder:
/// `None` when it passes, else the reason.
async fn integrity_problem(store_root: &Path, kind: ModelKind) -> Option<String> {
    crate::model::verify_captioner_dir_async(store_root, kind)
        .await
        .err()
        .map(|e| e.to_string())
}

/// [`captioner_statuses`] with the load-time integrity check applied to every
/// installed captioner that loads from a pinned snapshot folder, so the list
/// never offers one the pipeline would refuse. Small (code/config) files are
/// re-hashed per call; the weights' hash is cached by the check itself.
pub async fn captioner_statuses_verified(
    db: &Database,
    store_root: &Path,
) -> Result<Vec<CaptionerStatus>> {
    let mut out = Vec::with_capacity(CAPTIONERS.len());
    for status in captioner_statuses(db).await? {
        let problem = match pinned_kind_for_role(status.captioner.role) {
            Some(kind) if status.installed => integrity_problem(store_root, kind).await,
            _ => None,
        };
        out.push(match problem {
            Some(reason) => CaptionerStatus {
                installed: false,
                unusable: Some(reason),
                ..status
            },
            None => status,
        });
    }
    Ok(out)
}

/// The escalation model's state: are its files all in the library, and does
/// its pinned folder pass the same load-time check the pipeline runs before
/// loading it?
pub async fn escalation_status(db: &Database, store_root: &Path) -> Result<EscalationStatus> {
    let files_present = complete_dir_for_role(
        db,
        super::caption::QWEN_VL_ROLE,
        super::caption::QWEN_VL_REQUIRED_FILES,
    )
    .await?
    .is_some();
    let reason = if files_present {
        integrity_problem(store_root, ModelKind::QwenVlEngine).await
    } else {
        None
    };
    Ok(EscalationStatus {
        files_present,
        usable: files_present && reason.is_none(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;

    fn model_row(file_path: &str, role: &str) -> NewModel {
        NewModel {
            name: file_path.into(),
            format: "onnx".into(),
            file_path: file_path.into(),
            size_bytes: 1,
            source: "manual".into(),
            roles: vec![role.into()],
            ..NewModel::default()
        }
    }

    #[test]
    fn ids_are_unique_and_find_works() {
        let mut ids: Vec<&str> = CAPTIONERS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CAPTIONERS.len());
        assert_eq!(
            find_captioner(WD_TAGGER_ID).unwrap().style,
            CaptionStyle::Tags
        );
        assert!(find_captioner("nope").is_none());
    }

    #[test]
    fn only_prose_captioners_support_escalation() {
        for c in CAPTIONERS {
            assert_eq!(
                c.supports_escalation,
                c.style == CaptionStyle::Prose,
                "{}",
                c.id
            );
        }
    }

    #[test]
    fn wd_tagger_role_matches_its_model_kinds_default_role() {
        use crate::model::ModelKind;
        assert_eq!(ModelKind::WdTagger.default_role(), Some(WD_TAGGER_ROLE));
        assert_eq!(find_captioner(WD_TAGGER_ID).unwrap().role, WD_TAGGER_ROLE);
    }

    #[tokio::test]
    async fn statuses_reflect_which_roles_are_present_in_the_library() {
        let db = Database::connect_in_memory().await.unwrap();

        // Only the tag list -- not enough to call the tagger installed.
        db.models()
            .insert(model_row(
                "E:\\AI\\models\\vision\\wd-tagger\\selected_tags.csv",
                WD_TAGGER_ROLE,
            ))
            .await
            .unwrap();
        let s = captioner_statuses(&db).await.unwrap();
        let by_id = |s: &[CaptionerStatus], id: &str| {
            s.iter().find(|x| x.captioner.id == id).unwrap().installed
        };
        assert!(!by_id(&s, WD_TAGGER_ID), "tag list alone is not installed");
        assert!(!by_id(&s, FLORENCE2_ID));

        // Add the model file alongside it -- now both required files exist.
        db.models()
            .insert(model_row(
                "E:\\AI\\models\\vision\\wd-tagger\\model.onnx",
                WD_TAGGER_ROLE,
            ))
            .await
            .unwrap();
        let s = captioner_statuses(&db).await.unwrap();
        assert!(by_id(&s, WD_TAGGER_ID));
        assert!(!by_id(&s, FLORENCE2_ID));
    }

    #[tokio::test]
    async fn installed_captioner_dir_returns_the_directory_holding_all_required_files() {
        let db = Database::connect_in_memory().await.unwrap();
        let c = find_captioner(WD_TAGGER_ID).unwrap();

        db.models()
            .insert(model_row(
                "E:\\AI\\models\\vision\\wd-tagger-a\\model.onnx",
                WD_TAGGER_ROLE,
            ))
            .await
            .unwrap();
        db.models()
            .insert(model_row(
                "E:\\AI\\models\\vision\\wd-tagger-b\\selected_tags.csv",
                WD_TAGGER_ROLE,
            ))
            .await
            .unwrap();
        // Split across two directories -- neither has both required files.
        assert_eq!(installed_captioner_dir(&db, c).await.unwrap(), None);

        // Complete directory "a" -- it now qualifies.
        db.models()
            .insert(model_row(
                "E:\\AI\\models\\vision\\wd-tagger-a\\selected_tags.csv",
                WD_TAGGER_ROLE,
            ))
            .await
            .unwrap();
        let dir = installed_captioner_dir(&db, c).await.unwrap().unwrap();
        assert_eq!(dir, Path::new("E:\\AI\\models\\vision\\wd-tagger-a"));
    }

    /// A catalog stack install lands every member via `import_model` with
    /// its kind's default role in the kind's store folder -- detection must
    /// find exactly those directories for both one-click captioner stacks.
    #[tokio::test]
    async fn an_imported_captioner_stack_is_detected_by_its_kinds_role_and_folder() {
        use crate::model::{ModelKind, KNOWN_MODELS, MODEL_STACKS};

        let db = Database::connect_in_memory().await.unwrap();
        let store = Path::new("E:\\AI\\models");
        for stack_id in ["wd-tagger", "florence2-large", "qwen2.5-vl-7b"] {
            let stack = MODEL_STACKS.iter().find(|s| s.id == stack_id).unwrap();
            for id in stack.member_ids {
                let m = KNOWN_MODELS.iter().find(|m| &m.id == id).unwrap();
                let kind = ModelKind::from_hint(m.kind).unwrap();
                let path = store.join(kind.store_subdir()).join(m.file);
                let role = kind.default_role().unwrap();
                db.models()
                    .insert(model_row(&path.to_string_lossy(), role))
                    .await
                    .unwrap();
            }
        }

        let s = captioner_statuses(&db).await.unwrap();
        assert!(s.iter().all(|x| x.installed), "{s:?}");
        let wd = find_captioner(WD_TAGGER_ID).unwrap();
        assert_eq!(
            installed_captioner_dir(&db, wd).await.unwrap().unwrap(),
            store.join("vision/wd-tagger")
        );
        let florence = find_captioner(FLORENCE2_ID).unwrap();
        assert_eq!(
            installed_captioner_dir(&db, florence)
                .await
                .unwrap()
                .unwrap(),
            store.join("vision/florence2-large")
        );
        assert_eq!(
            complete_dir_for_role(
                &db,
                super::super::caption::QWEN_VL_ROLE,
                super::super::caption::QWEN_VL_REQUIRED_FILES
            )
            .await
            .unwrap()
            .unwrap(),
            store.join("vision/qwen2.5-vl-7b")
        );
    }

    /// Florence-2's loader reads the model *and* its processor (tokenizer,
    /// preprocessor config, remote code) from one directory, so a stack
    /// download that has landed only some files must not read as installed.
    #[tokio::test]
    async fn florence2_counts_as_installed_only_when_every_snapshot_file_is_present() {
        let db = Database::connect_in_memory().await.unwrap();
        let c = find_captioner(FLORENCE2_ID).unwrap();
        assert_eq!(installed_captioner_dir(&db, c).await.unwrap(), None);
        assert!(c.required_files.len() > 1, "{:?}", c.required_files);

        let (last, rest) = c.required_files.split_last().unwrap();
        for name in rest {
            db.models()
                .insert(model_row(
                    &format!("E:\\AI\\models\\vision\\florence2\\{name}"),
                    super::super::caption::FLORENCE2_ROLE,
                ))
                .await
                .unwrap();
        }
        assert_eq!(
            installed_captioner_dir(&db, c).await.unwrap(),
            None,
            "one file short of the snapshot is not installed"
        );

        db.models()
            .insert(model_row(
                &format!("E:\\AI\\models\\vision\\florence2\\{last}"),
                super::super::caption::FLORENCE2_ROLE,
            ))
            .await
            .unwrap();
        let dir = installed_captioner_dir(&db, c).await.unwrap().unwrap();
        assert_eq!(dir, Path::new("E:\\AI\\models\\vision\\florence2"));
    }

    /// The files a directory captioner needs are exactly the files its
    /// one-click catalog stack installs -- so "stack complete" and
    /// "captioner installed" can never disagree.
    #[test]
    fn required_files_are_exactly_the_files_of_each_catalog_stack() {
        use crate::model::{KNOWN_MODELS, MODEL_STACKS};

        let stack_files = |stack_id: &str| -> Vec<&'static str> {
            let stack = MODEL_STACKS.iter().find(|s| s.id == stack_id).unwrap();
            let mut files: Vec<&'static str> = stack
                .member_ids
                .iter()
                .map(|id| KNOWN_MODELS.iter().find(|m| &m.id == id).unwrap().file)
                .collect();
            files.sort_unstable();
            files
        };
        let sorted = |files: &[&'static str]| -> Vec<&'static str> {
            let mut v = files.to_vec();
            v.sort_unstable();
            v
        };

        assert_eq!(
            sorted(find_captioner(FLORENCE2_ID).unwrap().required_files),
            stack_files("florence2-large")
        );
        assert_eq!(
            sorted(find_captioner(WD_TAGGER_ID).unwrap().required_files),
            stack_files("wd-tagger")
        );
        assert_eq!(
            sorted(super::super::caption::QWEN_VL_REQUIRED_FILES),
            stack_files("qwen2.5-vl-7b")
        );
    }

    /// The captioner list the UI shows must not call a pinned captioner
    /// installed when its folder would fail the load-time check (here: the
    /// store folder does not exist at all), and it says why.
    #[tokio::test]
    async fn a_complete_but_unverifiable_pinned_captioner_is_listed_as_unusable() {
        let db = Database::connect_in_memory().await.unwrap();
        let store = tempfile::tempdir().unwrap();
        let florence = find_captioner(FLORENCE2_ID).unwrap();
        for name in florence.required_files {
            db.models()
                .insert(model_row(
                    &store
                        .path()
                        .join("vision/florence2-large")
                        .join(name)
                        .to_string_lossy(),
                    florence.role,
                ))
                .await
                .unwrap();
        }
        let wd = find_captioner(WD_TAGGER_ID).unwrap();
        for name in wd.required_files {
            db.models()
                .insert(model_row(
                    &store
                        .path()
                        .join("vision/wd-tagger")
                        .join(name)
                        .to_string_lossy(),
                    wd.role,
                ))
                .await
                .unwrap();
        }

        let s = captioner_statuses_verified(&db, store.path())
            .await
            .unwrap();
        let by_id = |id: &str| s.iter().find(|x| x.captioner.id == id).unwrap().clone();

        let fl = by_id(FLORENCE2_ID);
        assert!(!fl.installed, "{fl:?}");
        assert!(
            fl.unusable
                .as_deref()
                .is_some_and(|r| r.contains("captioner folder check failed")),
            "{fl:?}"
        );
        // Not a pinned snapshot kind: the tagger is judged by its files alone.
        let tagger = by_id(WD_TAGGER_ID);
        assert!(tagger.installed && tagger.unusable.is_none(), "{tagger:?}");
    }

    #[tokio::test]
    async fn escalation_status_separates_missing_from_unusable() {
        let db = Database::connect_in_memory().await.unwrap();
        let store = tempfile::tempdir().unwrap();

        let none = escalation_status(&db, store.path()).await.unwrap();
        assert_eq!(
            none,
            EscalationStatus {
                files_present: false,
                usable: false,
                reason: None
            }
        );

        for name in super::super::caption::QWEN_VL_REQUIRED_FILES {
            db.models()
                .insert(model_row(
                    &store
                        .path()
                        .join("vision/qwen2.5-vl-7b")
                        .join(name)
                        .to_string_lossy(),
                    super::super::caption::QWEN_VL_ROLE,
                ))
                .await
                .unwrap();
        }
        let present = escalation_status(&db, store.path()).await.unwrap();
        assert!(present.files_present);
        assert!(!present.usable, "{present:?}");
        assert!(present.reason.is_some(), "{present:?}");
    }
}
