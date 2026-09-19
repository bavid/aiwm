import type { HelpSetting, HelpTopic } from "./types.ts";

/** Chat: sessions, the model picker (llama.cpp and Colibri), personas,
 *  media commands, attached documents, compare. Facts from
 *  `features/chat/Chat.tsx`, `features/chat/personas/*`,
 *  `components/ChatSessionSidebar.tsx`, `components/CompareModels.tsx` and
 *  the persona / calibration notes in `docs/TODO.md`. */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "model",
    label: "Which model answers",
    what: "Auto, or one model from the library that carries the \"chat\" role.",
    why: "Auto picks the most-recently-used chat model on llama.cpp (or, with benchmark data and a preference set under Settings → Performance, the fastest or biggest one that fits). An explicit pick pins a model — including a Colibri model, which Auto never chooses.",
    effect: "The pick applies to the next message. A model that is not loaded yet is loaded first (the chat model and anything else on the GPU are swapped as the scheduler decides); a Colibri model runs on the CPU and RAM instead of the GPU.",
    benefit: "Leave it on Auto for everyday use; pin a model when you want a particular answer style or a model too big for the card.",
    pitfalls: "Only GGUF models with the chat role are listed — set the role on the Models tab if a model is missing here. A model that does not fit the card is refused with the reason on the turn.",
    measured: "2026-09-16: loading Mistral-Small-3.2-24B (IQ3_M, 8,192-token context) added 11,690 MB of VRAM; 2026-09-17 the same model answered at 56.35 tokens/s with a 6.1 s cold load.",
  },
  {
    key: "persona",
    label: "Persona",
    what: "A saved name, icon and system prompt that the model reads before your message. The chip shows which one is in effect and where it comes from (\"global\" or \"this chat\").",
    why: "A system prompt sets the tone for every answer — concise, tutor-like, pirate — without retyping it.",
    effect: "The prompt is sent verbatim as the system message ahead of your text; without a persona the request is exactly what it was before personas existed. The menu sets the global default (applies to every chat that inherits) and, for a named chat, an override: inherit, none, or a specific persona. The answer is stamped with the persona that produced it, so history still shows who answered after a rename or delete.",
    benefit: "One click switches the whole conversation's voice; different chats can have different voices.",
    pitfalls: "Ungrouped (\"Unsorted messages\") has no override — only the global default applies there. The prompt assistant on the Image, Video and Voice tabs ignores personas on purpose. Limits: name 1–60 characters, icon one emoji (64 bytes), prompt 1–8,000 characters. Deleting a persona sends the chats that used it back to the global default.",
    measured: "2026-09-18: the same question three times on Mistral-Small-3.2-24B — with a global \"Pirate\" persona the answer came in pirate speak; with the chat set to \"none\" it was plain; with no persona at all it was word-for-word the same plain answer, and the job carried no persona at all.",
  },
  {
    key: "compare",
    label: "Compare models",
    what: "Sends one prompt to two chat models at once and shows both answers side by side, with tokens and tokens/s under each.",
    why: "Reading two answers next to each other is the quickest way to decide which model to keep.",
    effect: "Two jobs are submitted. With one GPU they run one after the other (the second waits while the first is loaded and answers); a model that does not fit shows the reason instead of an answer. The comparison is not saved — closing the dialog loses it.",
    benefit: "Pick a model by its answers, not by its download count.",
    pitfalls: "Only shown when the library has at least two chat models.",
  },
  {
    key: "composer",
    label: "Message",
    what: "Your message, or a media command: /image <prompt>, /video <prompt> or /edit <instruction> at the start of the line.",
    why: "Generating a picture or a clip mid-conversation should not mean switching tabs.",
    effect: "A plain message goes to the chat model. /image submits an image job with the Image tab's defaults (1024×1024, 25 steps, CFG 7), /video a video job with the Video tab's defaults (832×480, 81 frames, 24 fps, 30 steps, CFG 5), /edit an edit of the most recent image in this chat (8 steps, CFG 1.5, FLUX.2 [klein]). The result appears inline. Enter sends, Shift+Enter inserts a newline.",
    benefit: "One thread holds the conversation and the pictures it produced.",
    pitfalls: "/edit needs an image in this chat first. Media results only count as part of a named chat — in \"Unsorted messages\" they show on the Image/Video tabs, not here. The chat sends only the current message, not the earlier turns (no conversation memory yet).",
  },
  {
    key: "documents",
    label: "Attach document",
    what: "Adds a .txt or .md file to the current chat; its text grounds the model's answers.",
    why: "Asking about a document you have is more useful than asking from memory.",
    effect: "The file is read, chunked and stored for this chat; answers are grounded by keyword (lexical) search over those chunks — no embedding model is involved. Documents belong to the chat: switch chats and the list switches with it. Remove drops the document from the chat, not the file from disk.",
    benefit: "Ask about your own notes, specs or transcripts.",
    pitfalls: "Only inside a named chat (not in \"Unsorted messages\"), and only from the desktop app — the browser preview has no file picker.",
  },
  {
    key: "sessions",
    label: "Chats",
    what: "The list on the left: New chat, rename, archive, delete, and \"Unsorted messages\" for turns that belong to no chat.",
    why: "Separate topics stay separate; an old chat can be put away without deleting it.",
    effect: "New chat creates a named chat (its first message becomes its name — the first three words — until you rename it). Archive hides a chat under \"Show archived\"; Delete removes the chat but keeps its messages in the history as unsorted. History is read from the jobs table, so it survives restarts.",
    benefit: "Conversations come back after a restart, and each one keeps its own persona and documents.",
  },
  {
    key: "delete-turn",
    label: "Delete (a message)",
    what: "Removes one question-and-answer pair from the history.",
    why: "A wrong turn or a test message does not have to stay in the record.",
    effect: "The job behind the turn is deleted from the database; an image or clip it produced is deleted with it.",
    benefit: "Keep the history readable.",
    pitfalls: "Not undoable.",
  },
];

export const CHAT_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "chat",
    title: "What the Chat tab is for",
    summary: "Talk to a local model on llama.cpp (GGUF) or Colibri (CPU, for models too big for the card), in named chats that survive restarts.",
    body: [
      {
        kind: "p",
        text: "Every message is a job: it is queued, the model is loaded if it is not resident, the answer streams in and is stored. The model, the tokens and the tokens per second show under each answer. Because past turns are read from the jobs table, a chat is still there after switching tabs or restarting the app.",
      },
    ],
  },
  {
    id: "flow",
    area: "chat",
    title: "Step by step",
    summary: "New chat, pick a persona if you like, type, read, and generate media without leaving.",
    body: [
      {
        kind: "steps",
        items: [
          "Press \"+ New chat\" (or stay in \"Unsorted messages\").",
          "Leave the model on Auto, or pick one. The first message loads it — that takes seconds to a minute for a large model.",
          "Optional: pick a persona from the chip. Global default for every chat, or an override for this one.",
          "Type and press Enter. Stop cancels an answer that is still streaming.",
          "Need a picture? /image a bay at dawn. A clip? /video a paper boat in the rain. Change the last picture? /edit make it night.",
          "Attach a .txt or .md to ask about it.",
        ],
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "chat",
    title: "What happens on disk and on the GPU",
    summary: "The chat model lives on the GPU while it answers; every turn is a row in the database; documents are indexed locally.",
    body: [
      {
        kind: "list",
        items: [
          "GPU: llama.cpp keeps the picked model loaded between messages so the next answer starts at once. It is unloaded when another job needs the room (or from the Dashboard). The context window defaults to 8,192 tokens (Settings → Runtimes).",
          "Colibri models run on the CPU with the weights streamed from disk; they need RAM, not VRAM, and are slower.",
          "Disk: turns, chats and personas are rows in the app database; /image and /video outputs go to the generated-media folder like any other render.",
          "Nothing is sent anywhere: the model runs on this machine.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "chat",
    title: "Limits and pitfalls",
    summary: "No conversation memory yet, one model at a time, VRAM decides.",
    body: [
      {
        kind: "list",
        items: [
          "Each message is sent on its own; the model does not see the earlier turns of the chat.",
          "One model answers at a time. Compare models runs its two jobs one after the other on one GPU.",
          "A model that does not fit the card is refused with the reason; a model that fits only when something else unloads is blocked until it does.",
          "Personas apply to llama.cpp and Colibri chats; not to agents, Story Studio or the prompt assistant.",
          "Documents are keyword-searched, not semantically; a question that uses different words than the document may miss.",
        ],
      },
    ],
  },
];
