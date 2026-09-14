export type AssistantKind = "image" | "video" | "edit" | "narrate";

export interface AssistantTurn {
  role: "user" | "assistant";
  text: string;
}

/** Frames the model's job for this session -- asked once, at the top of every
 *  request, since the underlying chat endpoint has no separate "system" turn
 *  and no server-side memory (see submission below): each request re-sends
 *  the whole transcript so far. */
export function systemPreambleFor(kind: AssistantKind): string {
  if (kind === "narrate") {
    return (
      "You are helping someone write a short line for a game-style off-screen narrator -- " +
      "read aloud by a local text-to-speech voice, not spoken by a character in the scene. " +
      "Have a short back-and-forth to understand the moment being narrated -- ask at most one " +
      "clarifying question at a time, keep replies brief. Write in full sentences with real " +
      "punctuation (periods, commas, em dashes): each sentence gets its own pause when spoken " +
      "aloud, so that's the pacing tool to lean on, not special syntax. Keep it to a couple of " +
      "sentences -- this narrator speaks a beat at a time, not a whole scene. As soon as you " +
      "have enough, end your reply with the line on its own line, exactly in this form:\n" +
      "PROMPT: <the narration line, ready to read aloud as plain text>"
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
): string {
  const lines = [systemPreambleFor(kind), ""];
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
