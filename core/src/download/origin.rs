//! What a download is — where it comes from and which base family it is made
//! for — carried through the queue so the import can record it on the model
//! (Plan 14: "downloads remember what they are").

use serde::Deserialize;

use crate::model::family::{family_for_civitai, family_for_hf, FamilySource};
use crate::{CoreError, Result};

/// Recorded on the imported model: `origin` becomes its `source` (when the
/// import left `manual` there) and `base_family` its base family, decided by
/// `family_source`. [`Default`] = nothing known (a catalog stack, an older
/// client).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DownloadOrigin {
    /// `civitai:<modelId>/<versionId>` or `hf:<repo>@<rev>`.
    pub origin: Option<String>,
    /// Registry id the source's base label maps to; `None` when the label is
    /// missing or unknown — never guessed.
    pub base_family: Option<&'static str>,
    pub family_source: Option<FamilySource>,
}

/// The source metadata a client sends with a download (the Discover card
/// already has it — nothing is parsed out of the URL).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DownloadOriginDto {
    /// `civitai` | `hf`.
    pub source: String,
    /// Civitai's numeric model id, or the Hugging Face `owner/repo`.
    pub model_id: String,
    /// Civitai's numeric version id, or the Hugging Face revision
    /// (`main` when omitted).
    #[serde(default)]
    pub version: Option<String>,
    /// Civitai's `baseModel` label, or Hugging Face's `base_model` id.
    #[serde(default)]
    pub base_model: Option<String>,
}

fn err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("download origin: {msg}"))
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `owner/repo` with the characters the Hub allows.
fn is_hf_repo(s: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    matches!(s.split_once('/'), Some((owner, repo)) if ok(owner) && ok(repo))
}

fn is_hf_revision(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
}

fn trimmed(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

impl DownloadOrigin {
    /// A Civitai model version. `base_label` is the version's `baseModel`.
    pub fn civitai(
        model_id: &str,
        version_id: Option<&str>,
        base_label: Option<&str>,
    ) -> Result<Self> {
        let model_id = model_id.trim();
        if !is_numeric(model_id) {
            return Err(err(format!("{model_id:?} is not a Civitai model id")));
        }
        let origin = match trimmed(version_id) {
            Some(v) if is_numeric(v) => format!("civitai:{model_id}/{v}"),
            Some(v) => return Err(err(format!("{v:?} is not a Civitai version id"))),
            None => format!("civitai:{model_id}"),
        };
        let family = trimmed(base_label).and_then(family_for_civitai);
        Ok(Self {
            origin: Some(origin),
            base_family: family.map(|f| f.id),
            family_source: family.map(|_| FamilySource::Civitai),
        })
    }

    /// A Hugging Face repo at a revision (`main` when `None`). `base_model`
    /// is the repo's `base_model` (bare or in tag form).
    pub fn hf(repo: &str, revision: Option<&str>, base_model: Option<&str>) -> Result<Self> {
        let repo = repo.trim();
        if !is_hf_repo(repo) {
            return Err(err(format!("{repo:?} is not a Hugging Face owner/repo")));
        }
        let revision = trimmed(revision).unwrap_or("main");
        if !is_hf_revision(revision) {
            return Err(err(format!("{revision:?} is not a revision")));
        }
        let family = trimmed(base_model).and_then(family_for_hf);
        Ok(Self {
            origin: Some(format!("hf:{repo}@{revision}")),
            base_family: family.map(|f| f.id),
            family_source: family.map(|_| FamilySource::Hf),
        })
    }

    /// From the API body; `None` → nothing known.
    pub fn from_dto(dto: Option<&DownloadOriginDto>) -> Result<Self> {
        let Some(dto) = dto else {
            return Ok(Self::default());
        };
        match dto.source.trim().to_ascii_lowercase().as_str() {
            "civitai" => Self::civitai(
                &dto.model_id,
                dto.version.as_deref(),
                dto.base_model.as_deref(),
            ),
            "hf" | "huggingface" => Self::hf(
                &dto.model_id,
                dto.version.as_deref(),
                dto.base_model.as_deref(),
            ),
            other => Err(err(format!("unknown source {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_civitai_pony_lora_records_its_origin_and_base() {
        let o = DownloadOrigin::civitai("12345", Some("67890"), Some("Pony")).unwrap();
        assert_eq!(o.origin.as_deref(), Some("civitai:12345/67890"));
        assert_eq!(o.base_family, Some("pony"));
        assert_eq!(o.family_source, Some(FamilySource::Civitai));
    }

    #[test]
    fn an_unknown_label_records_the_origin_but_no_family() {
        let o = DownloadOrigin::civitai("1", None, Some("ZImageBase")).unwrap();
        assert_eq!(o.origin.as_deref(), Some("civitai:1"));
        assert_eq!(o.base_family, None);
        assert_eq!(o.family_source, None);
    }

    #[test]
    fn a_hugging_face_repo_records_repo_revision_and_base() {
        let o = DownloadOrigin::hf(
            "someone/klein-style",
            None,
            Some("base_model:adapter:black-forest-labs/FLUX.2-klein-4B"),
        )
        .unwrap();
        assert_eq!(o.origin.as_deref(), Some("hf:someone/klein-style@main"));
        assert_eq!(o.base_family, Some("flux2-klein-4b"));
        assert_eq!(o.family_source, Some(FamilySource::Hf));
    }

    #[test]
    fn malformed_ids_are_refused() {
        assert!(DownloadOrigin::civitai("12a", None, None).is_err());
        assert!(DownloadOrigin::civitai("1", Some("x/y"), None).is_err());
        assert!(DownloadOrigin::hf("no-slash", None, None).is_err());
        assert!(DownloadOrigin::hf("a/b/c", None, None).is_err());
        assert!(DownloadOrigin::hf("a/b", Some("../etc"), None).is_err());
        let bad = DownloadOriginDto {
            source: "ftp".into(),
            model_id: "1".into(),
            version: None,
            base_model: None,
        };
        assert!(DownloadOrigin::from_dto(Some(&bad)).is_err());
    }

    #[test]
    fn no_dto_means_nothing_known() {
        assert_eq!(
            DownloadOrigin::from_dto(None).unwrap(),
            DownloadOrigin::default()
        );
    }
}
