//! Caption composition (spec 3A/3D): the exported caption is assembled from
//! separately stored parts — the dataset trigger, each assigned concept's
//! token (+ optional description), and the frame's own auto/hand caption —
//! in an order the training profile chooses (tags first for Anime/SDXL,
//! prose first for FLUX.2). Nothing here is ever written back into
//! `dataset_frames.caption`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionOrder {
    TagsFirst,
    ProseFirst,
}

/// What the auto caption is: a comma-separated tag list (WD tagger) or a
/// sentence (Florence-2 / JoyCaption). Carried through the call signature
/// now so the pipeline wiring (Task 10) can later join tag output as one more
/// tag block; today `compose_caption` treats both styles the same (the
/// caption is its own clause joined with ", ").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionStyle {
    Prose,
    Tags,
}

/// One assigned concept as it reaches composition.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptPart<'a> {
    pub token: &'a str,
    pub description: &'a str,
}

/// Build the final caption. Rules (all covered by the tests below):
/// - the trigger (if non-empty) always comes first;
/// - concept tokens follow, each as `token` or `token, description`;
/// - the auto/hand caption is placed after the tags (`TagsFirst`) or the
///   trigger+tokens are placed after it (`ProseFirst`);
/// - empty parts are skipped; the result never has leading/trailing
///   separators.
pub fn compose_caption(
    order: CaptionOrder,
    trigger: &str,
    concepts: &[ConceptPart<'_>],
    auto_caption: &str,
    _auto_style: CaptionStyle,
) -> String {
    let mut tag_block: Vec<String> = Vec::new();
    let trigger = trigger.trim();
    if !trigger.is_empty() {
        tag_block.push(trigger.to_string());
    }
    for c in concepts {
        let token = c.token.trim();
        if token.is_empty() {
            continue;
        }
        let description = c.description.trim();
        if description.is_empty() {
            tag_block.push(token.to_string());
        } else {
            // The token/description boundary is deliberately flattened: the
            // composed caption is only ever consumed by the trainer, never
            // parsed back, so a description that itself contains ", " is fine.
            tag_block.push(format!("{token}, {description}"));
        }
    }
    let tags = tag_block.join(", ");
    let prose = auto_caption.trim();
    match (tags.is_empty(), prose.is_empty(), order) {
        (true, true, _) => String::new(),
        (false, true, _) => tags,
        (true, false, _) => prose.to_string(),
        (false, false, CaptionOrder::TagsFirst) => format!("{tags}, {prose}"),
        (false, false, CaptionOrder::ProseFirst) => format!("{prose}, {tags}"),
    }
}

/// Words a trigger token must not be: they already mean something to the
/// base model, so training on them drags the whole concept along. Lowercase.
const COMMON_WORDS: &[&str] = &[
    "anime",
    "manga",
    "photo",
    "picture",
    "image",
    "art",
    "style",
    "character",
    "person",
    "man",
    "woman",
    "boy",
    "girl",
    "cat",
    "dog",
    "foot",
    "feet",
    "hand",
    "hands",
    "face",
    "eyes",
    "hair",
    "dress",
    "shirt",
    "car",
    "house",
    "tree",
    "sky",
    "night",
    "day",
    "red",
    "blue",
    "green",
    "black",
    "white",
];

/// `Some(warning)` when `token` looks like an ordinary word (or is empty /
/// contains spaces). The warning includes a concrete suggestion.
pub fn token_warning(token: &str) -> Option<String> {
    let t = token.trim();
    if t.is_empty() {
        return Some("Token is empty — use something like 'kenji_xy'.".into());
    }
    if t.chars().any(char::is_whitespace) {
        return Some(format!(
            "Token contains spaces — use one word, e.g. '{}_xy'.",
            t.split_whitespace()
                .collect::<Vec<_>>()
                .join("_")
                .to_lowercase()
        ));
    }
    let lower = t.to_lowercase();
    if COMMON_WORDS.contains(&lower.as_str()) {
        return Some(format!(
            "'{t}' is an ordinary word the base model already knows — use a made-up token like '{lower}_xy'."
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part<'a>(token: &'a str, description: &'a str) -> ConceptPart<'a> {
        ConceptPart { token, description }
    }

    #[test]
    fn tags_first_puts_trigger_then_tokens_then_prose() {
        let out = compose_caption(
            CaptionOrder::TagsFirst,
            "ghibli_xy",
            &[part("kenji_xy", "bare, visible"), part("pusemukkel_xy", "")],
            "a boy running across a field",
            CaptionStyle::Prose,
        );
        assert_eq!(
            out,
            "ghibli_xy, kenji_xy, bare, visible, pusemukkel_xy, a boy running across a field"
        );
    }

    #[test]
    fn prose_first_puts_the_caption_before_trigger_and_tokens() {
        let out = compose_caption(
            CaptionOrder::ProseFirst,
            "ghibli_xy",
            &[part("kenji_xy", "")],
            "a boy running across a field",
            CaptionStyle::Prose,
        );
        assert_eq!(out, "a boy running across a field, ghibli_xy, kenji_xy");
    }

    #[test]
    fn without_a_caption_only_trigger_and_tokens_remain_in_stable_order() {
        let out = compose_caption(
            CaptionOrder::ProseFirst,
            "ghibli_xy",
            &[part("b_xy", ""), part("a_xy", "")],
            "",
            CaptionStyle::Tags,
        );
        assert_eq!(out, "ghibli_xy, b_xy, a_xy");
    }

    #[test]
    fn everything_empty_yields_an_empty_caption_and_blank_parts_are_skipped() {
        assert_eq!(
            compose_caption(
                CaptionOrder::TagsFirst,
                " ",
                &[part("  ", "")],
                "  ",
                CaptionStyle::Prose
            ),
            ""
        );
        assert_eq!(
            compose_caption(
                CaptionOrder::TagsFirst,
                "",
                &[],
                "just prose",
                CaptionStyle::Prose
            ),
            "just prose"
        );
    }

    #[test]
    fn token_warning_flags_common_words_spaces_and_empty_but_not_made_up_tokens() {
        assert!(token_warning("anime").unwrap().contains("anime_xy"));
        assert!(token_warning("Foot").unwrap().contains("foot_xy"));
        assert!(token_warning("old man").unwrap().contains("old_man_xy"));
        assert!(token_warning("").is_some());
        assert_eq!(token_warning("kenji_xy"), None);
        assert_eq!(token_warning("pusemukkel"), None);
    }
}
