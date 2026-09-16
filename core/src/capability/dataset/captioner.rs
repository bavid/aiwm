//! Captioner registry (spec 3D): Florence-2 is one entry, not the pipeline.
//! Each captioner is resolved from the model library by role, exactly like
//! `caption::resolve_florence2_dir` always did; `installed` is what the
//! Dataset tab's "Beschreiben mit" dropdown filters on.

use serde::Serialize;

use super::compose::CaptionStyle;
use crate::db::Database;
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
    pub vram_mb: u64,
    pub license: &'static str,
    /// Whether the frame-X-vs-X+N temporal escalation applies on top of it.
    /// A tagger has no sentence to judge for confidence, so it cannot.
    pub supports_escalation: bool,
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
    },
    Captioner {
        id: WD_TAGGER_ID,
        name: "WD EVA02 Tagger v3 (Danbooru tags)",
        style: CaptionStyle::Tags,
        role: "vision_wd_tagger",
        vram_mb: 0, // onnxruntime on the CPU
        license: "Apache-2.0",
        supports_escalation: false,
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
    pub installed: bool,
}

pub async fn captioner_statuses(db: &Database) -> Result<Vec<CaptionerStatus>> {
    let mut out = Vec::with_capacity(CAPTIONERS.len());
    for c in CAPTIONERS {
        let installed = !db.models().for_role(c.role).await?.is_empty();
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
    fn every_captioner_role_is_a_default_role_of_some_model_kind() {
        use crate::model::ModelKind;
        for c in CAPTIONERS {
            if c.id == WD_TAGGER_ID {
                assert_eq!(ModelKind::WdTagger.default_role(), Some(c.role));
            }
        }
    }

    #[tokio::test]
    async fn statuses_reflect_which_roles_are_present_in_the_library() {
        let db = Database::connect_in_memory().await.unwrap();
        db.models()
            .insert(NewModel {
                name: "model.onnx".into(),
                format: "onnx".into(),
                file_path: "E:\\AI\\models\\vision\\wd-tagger\\model.onnx".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec!["vision_wd_tagger".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        let s = captioner_statuses(&db).await.unwrap();
        let by_id = |id: &str| s.iter().find(|x| x.captioner.id == id).unwrap().installed;
        assert!(by_id(WD_TAGGER_ID));
        assert!(!by_id(FLORENCE2_ID));
    }
}
