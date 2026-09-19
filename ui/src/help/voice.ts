import type { HelpSetting, HelpTopic } from "./types.ts";

/** Voice tab: the narrator — Kokoro and Dia engines, moods, voices, speed,
 *  saved voice identities (cloning), clean audio. Facts from
 *  `features/voice/Voice.tsx` and `components/VoiceIdentityPicker.tsx`. No
 *  measured timings exist in `docs/TODO.md` for narration, so none are
 *  quoted. */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "engine",
    label: "Narration engine",
    what: "Kokoro (fast, flat, 28 English voices with a speed control) or Dia (expressive: real non-verbal sounds like (laughs) and (sighs), multi-speaker [S1]/[S2] tags, voice cloning — slower, no speed control).",
    why: "The two are different tools: Kokoro reads lines cleanly and quickly; Dia performs them.",
    effect: "Switching resets the voice: Kokoro lands on the first mood preset, Dia on the free-text identity \"narrator\" with no saved voice picked. An engine whose files are not imported is disabled until they are (Models → Add models → Voice, \"Download entire stack\").",
    benefit: "Pick the engine per line: Kokoro for narration, Dia for a laugh or a sigh.",
    pitfalls: "Kokoro needs its voice model (.onnx) and voice data (.bin); Dia needs its 9 engine files and the 3-file DAC codec. Either set is enough to use the tab.",
  },
  {
    key: "preset",
    label: "Mood presets (Kokoro)",
    what: "Eight named moods — Ominous, Warm storyteller, Dry wit, Grand / epic, Cold and clinical, Wide-eyed wonder, Weary veteran, Playful trickster — each a real Kokoro voice plus a speed that suits it.",
    why: "Kokoro has no emotion control; a mood is a voice picked by ear and a matching pace, nothing more exotic.",
    effect: "Sets the Voice and Speed fields below. Changing either afterwards deselects the preset.",
    benefit: "A fitting narrator in one click, without auditioning 28 voices.",
  },
  {
    key: "voice-advanced",
    label: "Voice (advanced, Kokoro)",
    what: "One of Kokoro's 28 English voices by id — af_/am_ American female/male, bf_/bm_ British female/male.",
    why: "The presets cover eight; the other twenty are here.",
    effect: "The next Preview uses this voice; the mood preset is cleared. Only English voices are offered because the sidecar always phonemises as US English — a non-English voice would mispronounce the text.",
    benefit: "Find your own narrator.",
    pitfalls: "The accent/gender tag is the only thing about a voice that can be labelled without listening; audition with Preview.",
  },
  {
    key: "speed",
    label: "Speed (Kokoro)",
    what: "Playback pace from 0.50× to 2.00× in steps of 0.05.",
    why: "Pace is most of what \"mood\" is for a flat voice: 0.9 sounds grave, 1.1 eager.",
    effect: "Kokoro renders at that rate. Dia has no rate control, so the slider is hidden for it.",
    benefit: "Fine-tune a preset's pace to the line.",
  },
  {
    key: "voice-identity",
    label: "Voice (Dia)",
    what: "A saved voice identity — a short reference clip plus its transcript, saved once under a name — or \"Free-text identity (seed only)\".",
    why: "Dia has no named voices. Without a reference clip every render samples a random voice; a saved identity clones a chosen one.",
    effect: "With an identity picked, the render is conditioned on the clip: a real, chosen voice character. Without one, the free-text identity below is hashed into a stable seed, so repeats sound like the same speaker, but still a random one. The + saves a new identity; × deletes the picked one together with its reference clip.",
    benefit: "One narrator across every scene of a story.",
    pitfalls: "Deleting an identity removes its clip from disk. Identities are stored under the app's data folder.",
  },
  {
    key: "reference-clip",
    label: "Reference clip",
    what: "The audio file (wav, mp3, flac, ogg, m4a) a new voice identity is cloned from — 5–15 seconds, clean, one speaker.",
    why: "Cloning copies what it hears: background noise, music or a second voice end up in every narration.",
    effect: "The file is copied into the app's data folder when you save; the original stays where it is.",
    benefit: "A voice you chose, not one the model rolled.",
    pitfalls: "Browsing for a file needs the desktop app; in the browser preview, type the path.",
  },
  {
    key: "transcript",
    label: "Transcript",
    what: "Exactly what the reference clip says, word for word.",
    why: "Dia aligns the clip's audio with its text to learn the voice; a wrong transcript teaches the wrong sounds.",
    effect: "Stored with the identity and sent with every narration that uses it.",
    benefit: "A clean clone.",
  },
  {
    key: "narrator-identity",
    label: "Narrator identity (free text, Dia)",
    what: "A label like \"gravelly old man\" that is hashed into the random seed when no saved voice is picked.",
    why: "It keeps repeat narrations sounding like the same speaker without a reference clip.",
    effect: "Same text, same seed, same voice; a different label, a different random voice. It does not describe the voice — it only makes the roll repeatable.",
    benefit: "Consistency for free; cloning when you need a specific voice.",
  },
  {
    key: "clean-audio",
    label: "Clean audio",
    what: "A DSP pass on a finished clip: DC-offset removal, a gentle high-pass filter and spectral-gate noise reduction.",
    why: "Generated speech can carry a low rumble or a hiss under the voice.",
    effect: "The file is rewritten in place (same name, same URL); the player reloads it. Runs synchronously, not as a queued job; no model is involved.",
    benefit: "A cleaner clip without leaving the tab.",
    pitfalls: "Not undoable; run it once — a second pass has nothing left to fix.",
  },
];

export const VOICE_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "voice",
    title: "What the Voice tab is for",
    summary: "An off-screen narrator for Story Studio scenes — or any line — read by Kokoro or Dia, fully local.",
    body: [
      {
        kind: "p",
        text: "Text-to-speech runs in the app's Python sidecar, not in ComfyUI. Each Preview is a job of type \"tts\"; the result is an audio clip in the generated-media folder, listed under History with a player, Clean and Delete. The prompt assistant above the form can draft a line with a chat model — and, for Dia, suggest its real non-verbal tags.",
      },
    ],
  },
  {
    id: "flow",
    area: "voice",
    title: "Step by step",
    summary: "Import an engine once, pick engine and voice, write the line, Preview.",
    body: [
      {
        kind: "steps",
        items: [
          "Models → Add models → Voice: \"Download entire stack\" for Kokoro (fast) or Dia (expressive), or both. The tab shows a checklist until one engine is complete.",
          "Pick the engine.",
          "Kokoro: pick a mood, or a voice and speed. Dia: pick a saved voice, or type a narrator identity; save a new voice with + (clip + transcript).",
          "Write the line and press Preview. The clip plays when it is done and lands in History.",
          "Clean audio if it rumbles; Delete what you do not keep.",
        ],
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "voice",
    title: "What happens on disk and on the GPU",
    summary: "Engines are model files in the library; clips are files in the generated-media folder; identities keep a copy of their clip.",
    body: [
      {
        kind: "list",
        items: [
          "Kokoro is a small ONNX model with a voice-data file; Dia is 9 engine files plus the 3-file DAC codec. Both are imported into the model store like any model and loaded by the sidecar when a narration runs.",
          "Each Preview writes one audio file; Clean rewrites it in place; Delete removes the job and the file.",
          "A saved voice identity stores a copy of the reference clip and the transcript in the app's data folder; deleting the identity deletes that copy.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "voice",
    title: "Limits and pitfalls",
    summary: "English only, no emotion dial, Dia is slower, no timings measured.",
    body: [
      {
        kind: "list",
        items: [
          "Kokoro is phonemised as US English; its non-English voices are not offered.",
          "Kokoro has no emotion or tone control — a mood is a voice and a speed.",
          "Dia has no speed control and no freeform mood tags; real sounds like (laughs) or (sighs) are typed into the line.",
          "Without a saved voice, Dia's speaker is random but repeatable per identity label.",
          "No narration timings have been measured on this machine yet, so none are quoted here.",
        ],
      },
    ],
  },
];
