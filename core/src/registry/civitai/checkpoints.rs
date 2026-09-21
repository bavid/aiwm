//! "Which checkpoints exist for this base?" — the Civitai lookup behind a
//! package's findable base (Plan 14).
//!
//! `GET /api/v1/models?types=Checkpoint&baseModels=<label>&sort=Most
//! Downloaded&nsfw=<bool>`. Verified with one real unauthenticated request on
//! 2026-09-21 (`baseModels=Pony`, limit 3): Civitai honours the filter — all
//! three hits were Pony checkpoints (Pony Diffusion V6 XL, 1 088 654
//! downloads; CyberRealistic Pony; Pony Realism), every listed version
//! `baseModel: "Pony"`. The filter is model-level, though (Pony Diffusion's
//! own `baseModels` also lists `SD 1.5`), so the version is picked here by its
//! own `baseModel`, and hits without such a version are dropped.

use serde_json::Value;

use super::{parse_files, str_field, CivitaiSource};
use crate::registry::{CheckpointCandidate, RemoteFile};
use crate::Result;

/// How many hits to ask for: more than the caller keeps, so dropping a hit
/// whose versions target another base still leaves enough.
const FETCH: usize = 10;

/// The query parameters for one lookup.
pub(super) fn params(base_label: &str, nsfw: bool) -> Vec<(&'static str, String)> {
    vec![
        ("types", "Checkpoint".to_string()),
        ("baseModels", base_label.trim().to_string()),
        ("sort", "Most Downloaded".to_string()),
        ("limit", FETCH.to_string()),
        // Always explicit — see the module doc of `civitai`.
        ("nsfw", nsfw.to_string()),
    ]
}

pub(super) async fn search(
    source: &CivitaiSource,
    base_label: &str,
    nsfw: bool,
    limit: usize,
) -> Result<Vec<CheckpointCandidate>> {
    let body = source
        .get_json("/api/v1/models", &params(base_label, nsfw))
        .await?;
    Ok(parse(&body, base_label, nsfw, limit))
}

/// The top `limit` checkpoints by downloads whose some version targets
/// exactly `base_label`; NSFW-flagged models only when `nsfw` is on.
pub(super) fn parse(
    body: &Value,
    base_label: &str,
    nsfw: bool,
    limit: usize,
) -> Vec<CheckpointCandidate> {
    let label = base_label.trim();
    let mut hits: Vec<CheckpointCandidate> = body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| candidate(item, label))
        .filter(|c| nsfw || !c.nsfw)
        .collect();
    hits.sort_by_key(|c| std::cmp::Reverse(c.downloads));
    hits.truncate(limit);
    hits
}

fn candidate(item: &Value, label: &str) -> Option<CheckpointCandidate> {
    if str_field(item, "type").as_deref() != Some("Checkpoint") {
        return None;
    }
    let model_id = item.get("id").and_then(Value::as_u64)?.to_string();
    let version = item
        .get("modelVersions")
        .and_then(Value::as_array)?
        .iter()
        .find(|v| {
            str_field(v, "baseModel").is_some_and(|b| b.trim().eq_ignore_ascii_case(label))
        })?;
    let version_id = version.get("id").and_then(Value::as_u64)?.to_string();
    Some(CheckpointCandidate {
        model_id,
        version_id,
        name: str_field(item, "name").unwrap_or_else(|| "Checkpoint".to_string()),
        downloads: item
            .get("stats")
            .and_then(|s| s.get("downloadCount"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        base_model: str_field(version, "baseModel").unwrap_or_default(),
        nsfw: item.get("nsfw").and_then(Value::as_bool).unwrap_or(false),
        preview_image_url: version
            .get("images")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find_map(|i| str_field(i, "url"))),
        file: primary_file(version),
    })
}

/// The version's weight file: the first `Model` / `Pruned Model` file,
/// preferring a safetensors one.
fn primary_file(version: &Value) -> Option<RemoteFile> {
    let weights: Vec<Value> = version
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|raw| {
            matches!(
                str_field(raw, "type").as_deref(),
                Some("Model" | "Pruned Model")
            )
        })
        .cloned()
        .collect();
    let files = parse_files(&serde_json::json!({ "files": weights }));
    let safetensors = files.iter().position(|f| f.path.ends_with(".safetensors"));
    let pick = safetensors.or((!files.is_empty()).then_some(0))?;
    files.into_iter().nth(pick)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn item(id: u64, name: &str, downloads: i64, nsfw: bool, bases: &[(u64, &str)]) -> Value {
        json!({
            "id": id,
            "name": name,
            "type": "Checkpoint",
            "nsfw": nsfw,
            "stats": { "downloadCount": downloads },
            "modelVersions": bases.iter().map(|(vid, base)| json!({
                "id": vid,
                "baseModel": base,
                "images": [{ "url": format!("https://image.test/{vid}.jpeg") }],
                "files": [
                    { "name": "sdxl_vae.safetensors", "type": "VAE", "sizeKB": 1.0,
                      "hashes": { "SHA256": "A".repeat(64) } },
                    { "name": format!("{name}.ckpt"), "type": "Model", "sizeKB": 2.0 },
                    { "name": format!("{name}.safetensors"), "type": "Model", "sizeKB": 6_775_430.712_890_625,
                      "hashes": { "SHA256": "B".repeat(64) },
                      "downloadUrl": format!("https://civitai.com/api/download/models/{vid}") }
                ]
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn the_query_asks_for_checkpoints_of_the_label_by_downloads() {
        let p = params(" Pony ", false);
        assert!(p.contains(&("types", "Checkpoint".into())));
        assert!(p.contains(&("baseModels", "Pony".into())));
        assert!(p.contains(&("sort", "Most Downloaded".into())));
        assert!(p.contains(&("nsfw", "false".into())));
    }

    #[test]
    fn the_top_hits_by_downloads_with_a_matching_version_are_kept() {
        let body = json!({ "items": [
            item(2, "CyberRealistic Pony", 773_473, false, &[(20, "Pony")]),
            item(1, "Pony Diffusion V6 XL", 1_088_654, false, &[(10, "Pony")]),
            // Model-level filter hit whose versions target another base.
            item(3, "Some SDXL", 9_000_000, false, &[(30, "SDXL 1.0")]),
            item(4, "Pony Realism", 568_037, false, &[(41, "SDXL 1.0"), (40, "Pony")]),
            item(5, "Fourth", 1_000, false, &[(50, "Pony")]),
        ]});
        let got = parse(&body, "Pony", false, 3);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Pony Diffusion V6 XL",
                "CyberRealistic Pony",
                "Pony Realism"
            ]
        );
        assert_eq!(got[2].version_id, "40", "the version that targets Pony");
        let file = got[0].file.as_ref().unwrap();
        assert_eq!(file.path, "Pony Diffusion V6 XL.safetensors");
        assert_eq!(file.size, 6_938_041_050);
        assert_eq!(file.sha256.as_deref(), Some("b".repeat(64).as_str()));
    }

    #[test]
    fn nsfw_models_are_dropped_unless_asked_for() {
        let body = json!({ "items": [
            item(1, "Safe", 10, false, &[(10, "Pony")]),
            item(2, "Spicy", 20, true, &[(20, "Pony")]),
        ]});
        assert_eq!(parse(&body, "Pony", false, 3).len(), 1);
        assert_eq!(parse(&body, "Pony", true, 3)[0].name, "Spicy");
    }
}
