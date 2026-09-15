export type AssistantKind = "image" | "video" | "edit" | "narrate";

/** Which sidecar engine will actually read the narration back -- only
 *  meaningful for `kind: "narrate"`. Changes what the preamble tells the
 *  model is real: Dia understands a small vocabulary of real non-verbal
 *  tags and speaker turns that Kokoro would just read aloud literally, so
 *  suggesting them only makes sense when Dia is the one rendering. */
export type NarrateEngine = "kokoro" | "dia";

export interface AssistantTurn {
  role: "user" | "assistant";
  text: string;
}

/** Dia's real, recognized non-verbal tags (mirrors
 *  `aiwm_sidecar.dia._DIA_NONVERBAL_TAGS`) -- genuine input the model
 *  performs, not a freeform emotion/tone control. Kept in sync by hand;
 *  both lists are small and change together deliberately. */
const DIA_NONVERBAL_TAGS = [
  "laughs",
  "clears throat",
  "sighs",
  "gasps",
  "coughs",
  "singing",
  "sings",
  "mumbles",
  "beep",
  "groans",
  "sniffs",
];

/** Frames the model's job for this session -- asked once, at the top of every
 *  request, since the underlying chat endpoint has no separate "system" turn
 *  and no server-side memory (see submission below): each request re-sends
 *  the whole transcript so far. `narrateEngine` only matters for
 *  `kind: "narrate"` and defaults to Kokoro -- existing callers that don't
 *  pass it keep the exact wording they always had. */
export function systemPreambleFor(
  kind: AssistantKind,
  narrateEngine: NarrateEngine = "kokoro",
): string {
  if (kind === "narrate") {
    const dia =
      narrateEngine === "dia"
        ? "This narrator is Dia, which additionally understands a small, fixed set of real " +
          `non-verbal sound tags -- ${DIA_NONVERBAL_TAGS.map((t) => `(${t})`).join(", ")} -- ` +
          "genuinely performed, not just discarded like the pause markers above. Use them only " +
          "where the scene actually calls for that sound (a laugh, a sigh, someone clearing " +
          "their throat), not as a substitute for describing mood in the words themselves. " +
          "Still no freeform tag beyond this exact list and the pause markers -- \"(angry)\" is " +
          "just as unsupported for Dia as for any other voice.\n\n"
        : "";
    return (
      "You are helping someone write a short line (or a few beats) for a game-style " +
      "off-screen narrator -- read aloud by a local text-to-speech voice, not spoken by a " +
      "character in the scene. Keep the vivid, concrete details from what they describe " +
      "(specific creatures, objects, sensations) instead of flattening it into a generic " +
      "summary -- their description is the material, not just a mood to paraphrase.\n\n" +
      "If you still need to know something to write it well, ask exactly one short " +
      "clarifying question and STOP THERE -- do not also include a PROMPT: line in that same " +
      "reply; only finalize once you're not asking anything.\n\n" +
      "When you do finalize: write in full sentences with real punctuation (periods, commas, " +
      "em dashes) -- each sentence gets its own natural pause when spoken, so that's the main " +
      "pacing tool, not special syntax. On top of that, exactly these pause markers are real " +
      "and will produce an actual timed silence when spoken: (pause), (beat), (breath), " +
      "(long pause), (dramatic pause). Do not invent any other bracketed direction -- no " +
      "(angry), (whispering), (mysterious tone), or similar: no voice here can perform an " +
      "emotion from a tag, so anything else in parentheses is just discarded before narration, " +
      "never spoken and never changing the delivery. If a beat needs a different mood, write " +
      "it into the words themselves.\n\n" +
      dia +
      "A scene with several distinct beats can become several PROMPT: lines -- one beat per " +
      "line, in order; a single moment should stay one line. End your reply with those lines, " +
      "each on its own, exactly in this form:\n" +
      "PROMPT: <a narration beat, ready to read aloud as plain text>"
    );
  }
  if (kind === "edit") {
    return (
      "You are helping someone describe how they want an existing photo edited (e.g. " +
      '"remove the blisters", "make the hair blonde", "put me on a train platform", ' +
      '"make me look like Neo from The Matrix"). Have a short back-and-forth to understand ' +
      "exactly what they want changed -- ask at most one clarifying question at a time, keep " +
      "replies brief. As soon as you have enough, end your reply with the edit instruction on " +
      "its own line, exactly in this form:\n" +
      "PROMPT: <one clear instruction describing the edit, referring to “the image”>"
    );
  }
  const medium = kind === "image" ? "image" : "video";
  const extra =
    kind === "video"
      ? " Video prompts should describe motion and camera movement, not just a static scene."
      : "";
  return (
    `You are a prompt-writing assistant for an AI ${medium} generator. Have a short back-and-forth ` +
    `with the user to understand what they want to create -- ask at most one clarifying question at a ` +
    `time, keep replies brief.${extra} As soon as you have enough to propose a prompt, ` +
    `end your reply with it on its own lines, exactly in this form (omit NEGATIVE if not useful):\n` +
    `PROMPT: <the full image/video generation prompt>\n` +
    `NEGATIVE: <a short negative prompt>`
  );
}

/** llama.cpp's `/v1/chat/completions` here is sent one stateless user message
 *  per request (see `core::runtime::llamacpp::client::complete_stream`) --
 *  no history, no system role. So the whole transcript is flattened into
 *  that single message, with the model's own reply left open at the end for
 *  it to continue. */
export function buildTranscriptPrompt(
  kind: AssistantKind,
  history: AssistantTurn[],
  newMessage: string,
  narrateEngine: NarrateEngine = "kokoro",
): string {
  const lines = [systemPreambleFor(kind, narrateEngine), ""];
  for (const turn of history) {
    lines.push(`${turn.role === "user" ? "User" : "Assistant"}: ${turn.text}`);
  }
  lines.push(`User: ${newMessage}`, "Assistant:");
  return lines.join("\n");
}

/** Every occurrence of one marker (`PROMPT` or `NEGATIVE`) in an assistant
 *  reply, each bounded to stop at the *next* marker line of either kind (not
 *  just the other one) -- otherwise a reply with two `PROMPT:` lines lets the
 *  first capture swallow the second one whole, literal "PROMPT:" text and
 *  all. A model asked for one line sometimes still hands back several (e.g.
 *  a multi-beat narration); returning all of them lets the caller decide
 *  whether to join them instead of silently mangling the text. */
function extractAllMarked(answer: string, marker: "PROMPT" | "NEGATIVE"): string[] {
  const re = new RegExp(
    `(?:^|\\n)\\s*${marker}:\\s*([^\\n]*(?:\\n(?!\\s*(?:PROMPT|NEGATIVE):)[^\\n]*)*)`,
    "gi",
  );
  const out: string[] = [];
  for (const m of answer.matchAll(re)) {
    const v = m[1]?.trim();
    if (v) out.push(v);
  }
  return out;
}

/** Pulls a `PROMPT:` / `NEGATIVE:` suggestion out of an assistant reply, if
 *  it followed the format asked for in the preamble. Multiple `PROMPT:` (or
 *  `NEGATIVE:`) lines are joined rather than dropped -- e.g. a narration
 *  reply that comes back as several beats reads naturally as one line per
 *  sentence once joined, since sentence-split synthesis already turns that
 *  into real pauses. */
export function extractSuggestion(answer: string): { prompt?: string; negative?: string } {
  const prompts = extractAllMarked(answer, "PROMPT");
  const negatives = extractAllMarked(answer, "NEGATIVE");
  return {
    ...(prompts.length ? { prompt: prompts.join(" ") } : {}),
    ...(negatives.length ? { negative: negatives.join(", ") } : {}),
  };
}
