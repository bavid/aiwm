import type { HelpSetting, HelpTopic } from "./types.ts";

/** Agents tab: the start-here checklist, the runtime cards, profiles
 *  (OpenCode / Hermes), sessions with approvals, the external terminal
 *  launcher. Facts from `features/agents/*` (`Agents.tsx`, `StartHere.tsx`,
 *  `RuntimeCards.tsx`, `ProfileList.tsx`, `NewProfileForm.tsx`,
 *  `SessionPanel.tsx`, `TranscriptView.tsx`, `LauncherPanel.tsx`) and the E2E
 *  notes in `docs/TODO.md` (2026-09-12). */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "start-here",
    label: "Start here",
    what: "The checklist at the top of the tab: a coding model, an installed runtime, a profile — the three things that must exist before an agent can run.",
    why: "Each one lives on a different surface (the Models tab, a runtime card, the profile form), so a first visit otherwise opens on a wall of controls with no obvious first step.",
    effect: "A step that is missing says what to do and, for the profile, opens the form. Once all three hold, the checklist collapses to one line naming the next action.",
    benefit: "One place that answers \"what is stopping me\" without reading the whole page.",
  },
  {
    key: "runtime-status",
    label: "Runtime status",
    what: "One card per agent runtime: installed or not, what it is, and how many of your coding models it can actually drive.",
    why: "Both facts decide whether a session can start, and both used to be visible only inside the collapsed profile form.",
    effect: "A missing runtime shows its install step — \"Install Hermes\" for Hermes, the npm line for OpenCode. The model line counts the coding models, and for Hermes counts how many reach its context floor.",
    benefit: "The state of the two runtimes is readable at a glance, before you fill in a form.",
    pitfalls: "Hermes needs at least 64,000 tokens of context, for its main model and its auxiliary compression model alike; the Qwen2.5-Coder 14B models tested here are 32K and do not meet it (2026-09-12). A profile still saves, but the session errors out immediately.",
  },
  {
    key: "transcript",
    label: "Transcript",
    what: "The scrolling record of a session: the agent's messages, each tool call with its command and output, each approval prompt and what you answered.",
    why: "What the agent did matters more than what it said, so tool calls are cards rather than prose.",
    effect: "Blocks carry the time they started and who they came from. A tool's output folds after 12 lines with a button to show all of it. An answered approval keeps its decision on screen — \"Allowed once\", \"Allowed for this session\" or \"Denied\" — instead of only fading.",
    benefit: "A long run stays scannable: you can find the one command that failed without scrolling through a file dump.",
  },
  {
    key: "runtime",
    label: "Runtime",
    what: "Which agent program drives the session: OpenCode or Hermes.",
    why: "They are different tools with different requirements: OpenCode is a Node program you install yourself (opencode-ai on npm, on your PATH); Hermes is a Python service the app can install for you.",
    effect: "A runtime that is not installed disables Create and says so; its card above has the install step. \"Install Hermes\" pulls about 120 packages plus its own toolchain — a few minutes, with a progress line.",
    benefit: "Pick the tool; the app runs it against your local model.",
    pitfalls: "Hermes requires a model with at least 64,000 tokens of context — for its main model and its auxiliary compression model alike. The Qwen2.5-Coder 14B models tested here are 32K and do not meet it (2026-09-12); OpenCode has no such requirement and works after the context fix.",
  },
  {
    key: "coding-model",
    label: "Coding model",
    what: "Auto (the most-recently-used model with the \"coding\" role) or a specific one.",
    why: "Agents need a model that follows tool-calling instructions; the coding role marks the ones meant for it.",
    effect: "The session's model is loaded on llama.cpp with the model's full context, not the chat default of 8,192 tokens — a tool-calling system prompt alone needs 20–25k.",
    benefit: "One model for chat, another for agents, without re-importing.",
    pitfalls: "No model with the coding role? Import a coding GGUF (Qwen2.5-Coder for OpenCode, Hermes-3 for Hermes) and tick \"coding\" on the Models tab.",
  },
  {
    key: "workspace",
    label: "Workspace folder",
    what: "The one folder the agent may edit.",
    why: "Every edit stays inside it; that is the sandbox.",
    effect: "The runtime is started with this folder as its working directory; edits outside it are refused.",
    benefit: "Point it at one repo and nothing else is touched.",
  },
  {
    key: "extra-folders",
    label: "Extra read-only folders",
    what: "Additional folders the agent may read (comma- or newline-separated), never edit.",
    why: "A shared library or a spec next to the repo is often needed for context.",
    effect: "Listed on the profile as \"+ reads …\"; reads are allowed there, writes are not.",
    benefit: "Context without widening the sandbox.",
  },
  {
    key: "approvals",
    label: "Allow once / Always",
    what: "An inline approval prompt for every command the agent wants to run and every file it wants to edit.",
    why: "The agent may run shell commands; you decide, per command, whether it does.",
    effect: "Allow once permits this one request. Always permits the pattern the runtime proposes (shown on the button) for the rest of the session. Until you answer, the composer is disabled. The agent has no network access.",
    benefit: "Full control over what runs, with a fast path for repetitive safe commands.",
  },
  {
    key: "session",
    label: "Session",
    what: "One conversation with the agent in the chosen workspace: transcript, tool calls with their command and output, approvals, a composer, Stop.",
    why: "The transcript is where you see what the agent did, not only what it said.",
    effect: "Start session brings up the runtime and its model; the status line says whose turn it is (\"Starting\", \"Working\", \"Needs your approval\", \"Your turn\"); Stop session ends it. One session runs at a time — the profile rows say so instead of going quietly dead.",
    benefit: "A coding agent on your own model, on your own machine.",
    pitfalls: "Turns can take longer than 30 seconds; the adapters stream over a dedicated connection so a long turn no longer counts as a dead runtime (fixed 2026-09-12).",
  },
  {
    key: "launcher",
    label: "Launch external terminal",
    what: "Opens a real, independent terminal window running OpenCode or Hermes directly against a pinned local model in a workspace folder.",
    why: "Some people want the tool's own interface, not the embedded transcript.",
    effect: "The model is loaded and kept for the terminal; you drive the tool yourself. The terminal keeps running after AIWM closes — but the model server does not, so closing AIWM leaves the terminal with a dead connection. \"Stop & release model\" frees the model.",
    benefit: "The native tool, with the model management done for you.",
    pitfalls: "Only one external launch at a time; Hermes must be installed first (see its runtime card).",
  },
];

export const AGENTS_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "agents",
    title: "What the Agents tab is for",
    summary: "Run a coding agent — OpenCode or Hermes — on a local model, sandboxed to one workspace folder, with every command and edit waiting for your approval.",
    body: [
      {
        kind: "p",
        text: "A profile binds a runtime, a workspace, optional read-only folders and a coding model. A session starts that runtime with that model on llama.cpp, streams its transcript into the panel, and pauses on an approval prompt whenever the agent wants to run a command or edit a file. The launcher is the alternative: the same tools in their own terminal window.",
      },
    ],
  },
  {
    id: "flow",
    area: "agents",
    title: "Step by step",
    summary: "Import a coding model, create a profile, start a session, approve or refuse, stop.",
    body: [
      {
        kind: "steps",
        items: [
          "Models tab: import a coding GGUF and give it the \"coding\" role (the Code catalogue tab does this on download).",
          "Install the runtime: OpenCode yourself (npm), Hermes from its runtime card.",
          "+ New profile: name, runtime, coding model, workspace folder, optional extra read-only folders. Create.",
          "Start session. Wait for \"Your turn\", then type what you want done — Enter sends.",
          "Answer each approval prompt: Allow once, allow the shown pattern for the session, or Deny.",
          "Stop session ends it; Close transcript clears the panel. Delete removes a profile, after a confirm.",
        ],
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "agents",
    title: "What happens on disk and on the GPU",
    summary: "The model on llama.cpp at full context; edits only inside the workspace; Hermes as its own Python install.",
    body: [
      {
        kind: "list",
        items: [
          "GPU: the coding model is loaded on llama.cpp with its full context window, which needs more VRAM for the KV cache than a chat with 8,192 tokens. It stays loaded for the session; other jobs wait or evict it afterwards.",
          "Disk: the agent writes only inside the workspace (plus reads in the extra folders). Hermes is installed by the app into its own folder (a uv environment with the pinned hermes-agent); OpenCode is wherever npm put it.",
          "Profiles, sessions and transcripts are rows in the database and part of a backup.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "agents",
    title: "Limits and pitfalls",
    summary: "Context requirements, one session at a time, no network.",
    body: [
      {
        kind: "list",
        items: [
          "Hermes needs a ≥ 64K-context model; none of the tested 32K models qualifies. OpenCode does not have this requirement.",
          "One embedded session and one external launch at a time.",
          "The agent has no network access from the sandbox.",
          "Personas do not apply to agent sessions.",
          "Closing AIWM kills the model server behind an external terminal.",
        ],
      },
    ],
  },
];
