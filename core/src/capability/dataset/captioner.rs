//! Captioner registry (spec 3D): Florence-2 is one entry, not the pipeline.
//! Each captioner is resolved from the model library by role, exactly like
//! `caption::resolve_captioner_dir` does for every entry; `installed` is what the
//! Dataset tab's "Beschreiben mit" dropdown filters on.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::compose::CaptionStyle;
use crate::db::Database;
use crate::model::WD_TAGGER_ROLE;
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
    /// be usable; empty = any model row carrying the role counts (whole-
    /// directory snapshots like Florence-2).
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
        required_files: &[],
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
    pub installed: bool,
}

/// The directory that satisfies `c` — the parent of the first model row
/// carrying its role when `required_files` is empty (Florence-2: any single
/// imported row is the whole snapshot), or the first (sorted, for
/// determinism) directory among that role's rows whose sibling file names
/// cover every entry in `required_files`. `None` when no directory
/// qualifies, including when the role has no rows at all.
pub async fn installed_captioner_dir(db: &Database, c: &Captioner) -> Result<Option<PathBuf>> {
    let rows = db.models().for_role(c.role).await?;

    if c.required_files.is_empty() {
        return Ok(rows
            .first()
            .and_then(|m| Path::new(&m.file_path).parent())
            .map(Path::to_path_buf));
    }

    let mut dirs: Vec<PathBuf> = rows
        .iter()
        .filter_map(|m| Path::new(&m.file_path).parent())
        .map(Path::to_path_buf)
        .collect();
    dirs.sort();
    dirs.dedup();

    for dir in dirs {
        let names: Vec<String> = rows
            .iter()
            .filter_map(|m| Path::new(&m.file_path).parent().map(|p| (p, m)))
            .filter(|(p, _)| *p == dir)
            .filter_map(|(_, m)| {
                Path::new(&m.file_path)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .map(str::to_string)
            })
            .collect();
        let has_all = c
            .required_files
            .iter()
            .all(|req| names.iter().any(|n| n == req));
        if has_all {
            return Ok(Some(dir));
        }
    }
    Ok(None)
}

pub async fn captioner_statuses(db: &Database) -> Result<Vec<CaptionerStatus>> {
    let mut out = Vec::with_capacity(CAPTIONERS.len());
    for c in CAPTIONERS {
        let installed = installed_captioner_dir(db, c).await?.is_some();
        out.push(CaptionerStatus {
            captioner: *c,
            installed,
        });
    }
    Ok(out)
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
            super::super::caption::resolve_qwen_vl_dir(&db)
                .await
                .unwrap(),
            store.join("vision/qwen2.5-vl-7b")
        );
    }

    #[tokio::test]
    async fn florence2_counts_as_installed_with_any_single_row_carrying_its_role() {
        let db = Database::connect_in_memory().await.unwrap();
        let c = find_captioner(FLORENCE2_ID).unwrap();
        assert_eq!(installed_captioner_dir(&db, c).await.unwrap(), None);

        db.models()
            .insert(model_row(
                "E:\\AI\\models\\vision\\florence2\\anything.bin",
                super::super::caption::FLORENCE2_ROLE,
            ))
            .await
            .unwrap();
        let dir = installed_captioner_dir(&db, c).await.unwrap().unwrap();
        assert_eq!(dir, Path::new("E:\\AI\\models\\vision\\florence2"));
    }
}
