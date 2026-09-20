/** The in-app help content model (Plan 12).
 *
 *  One data source feeds both the Help tab and the `?` hints next to
 *  controls, so the two can never say different things. Plain structured text
 *  — no markdown renderer, no i18n, English like the rest of the UI.
 *
 *  These modules are also loaded by `scripts/check-help.mjs` under plain
 *  Node (type stripping), so they must stay free of React and of anything
 *  that needs a bundler: `import type` only, relative imports with a `.ts`
 *  extension, no enums. */

/** One documented area of the app. Each area is a section of the Help tab. */
export type HelpArea =
  | "getting-started"
  | "dashboard"
  | "chat"
  | "image"
  | "video"
  | "upscale"
  | "voice"
  | "stories"
  | "dataset"
  | "training"
  | "jobs"
  | "agents"
  | "models"
  | "benchmark"
  | "diagnostics"
  | "settings"
  | "shortcuts";

export const HELP_AREA_LABEL: Record<HelpArea, string> = {
  "getting-started": "Getting started",
  dashboard: "Dashboard",
  chat: "Chat",
  image: "Image",
  video: "Video",
  upscale: "Upscale",
  voice: "Voice",
  stories: "Stories",
  dataset: "Dataset",
  training: "Training",
  jobs: "Jobs",
  agents: "Agents",
  models: "Models & Discover",
  benchmark: "Benchmark",
  diagnostics: "Diagnostics",
  settings: "Settings",
  shortcuts: "Shortcuts",
};

/** A paragraph, an ordered list of steps, or a bullet list. */
export type HelpBlock =
  | { kind: "p"; text: string }
  | { kind: "steps"; items: readonly string[] }
  | { kind: "list"; items: readonly string[] };

/** One control, explained the way the backlog asks: what it does, why you
 *  need it, what happens when you change or start it, and what you gain.
 *  `measured` quotes a real number from `docs/TODO.md`, with its date. */
export interface HelpSetting {
  /** Unique within its area; the `<HelpHint setting=…>` reference. Only
   *  `[a-z0-9-]`, since it becomes part of a DOM id. */
  key: string;
  /** The control's label as it appears on screen. */
  label: string;
  what: string;
  why: string;
  effect: string;
  benefit: string;
  pitfalls?: string;
  measured?: string;
}

export interface HelpTopic {
  /** Unique within its area. Only `[a-z0-9-]`. */
  id: string;
  area: HelpArea;
  title: string;
  summary: string;
  body: readonly HelpBlock[];
  settings?: readonly HelpSetting[];
}

export interface HelpAreaContent {
  id: HelpArea;
  label: string;
  topics: readonly HelpTopic[];
}
