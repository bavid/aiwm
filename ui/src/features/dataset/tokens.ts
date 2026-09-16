/** Client-side mirror of `core/src/capability/dataset/compose.rs::token_warning`.
 *
 *  The server stays the source of truth — `DatasetConcept.token_warning` is
 *  what the concept list renders. This copy exists only so the create form can
 *  warn while the curator is still typing, before any round trip. Keep the word
 *  list below in sync with `COMMON_WORDS` in that module. */
const COMMON_WORDS: readonly string[] = [
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

/** A warning when `token` looks like an ordinary word (or is empty / contains
 *  spaces), `null` when it reads as a made-up token. Includes a concrete
 *  suggestion, same wording as the Rust side. */
export function tokenWarning(token: string): string | null {
  const t = token.trim();
  if (t === "") {
    return "Token is empty — use something like 'kenji_xy'.";
  }
  if (/\s/.test(t)) {
    const joined = t.split(/\s+/).join("_").toLowerCase();
    return `Token contains spaces — use one word, e.g. '${joined}_xy'.`;
  }
  const lower = t.toLowerCase();
  if (COMMON_WORDS.includes(lower)) {
    return `'${t}' is an ordinary word the base model already knows — use a made-up token like '${lower}_xy'.`;
  }
  return null;
}
