import { AGENTS_TOPICS } from "./agents.ts";
import { BENCHMARK_TOPICS } from "./benchmark.ts";
import { CHAT_TOPICS } from "./chat.ts";
import { DASHBOARD_TOPICS } from "./dashboard.ts";
import { DATASET_CURATION_TOPICS } from "./dataset-curation.ts";
import { DATASET_TOPICS } from "./dataset.ts";
import { DIAGNOSTICS_TOPICS } from "./diagnostics.ts";
import { GETTING_STARTED_TOPICS } from "./getting-started.ts";
import { IMAGE_TOPICS } from "./image.ts";
import { JOBS_TOPICS } from "./jobs.ts";
import { MODELS_TOPICS } from "./models.ts";
import { SETTINGS_TOPICS } from "./settings.ts";
import { SHORTCUTS_TOPICS } from "./shortcuts.ts";
import { STORIES_TOPICS } from "./stories.ts";
import { TRAINING_TOPICS } from "./training.ts";
import { UPSCALE_TOPICS } from "./upscale.ts";
import { VIDEO_TOPICS } from "./video.ts";
import { VOICE_TOPICS } from "./voice.ts";
import {
  HELP_AREA_LABEL,
  type HelpArea,
  type HelpAreaContent,
  type HelpSetting,
  type HelpTopic,
} from "./types.ts";

export type { HelpArea, HelpAreaContent, HelpBlock, HelpSetting, HelpTopic } from "./types.ts";
export { HELP_AREA_LABEL } from "./types.ts";

/** Every documented area, in Help-tab order. */
export const HELP_AREAS: readonly HelpAreaContent[] = [
  { id: "getting-started", label: HELP_AREA_LABEL["getting-started"], topics: GETTING_STARTED_TOPICS },
  { id: "dashboard", label: HELP_AREA_LABEL.dashboard, topics: DASHBOARD_TOPICS },
  { id: "chat", label: HELP_AREA_LABEL.chat, topics: CHAT_TOPICS },
  { id: "image", label: HELP_AREA_LABEL.image, topics: IMAGE_TOPICS },
  { id: "video", label: HELP_AREA_LABEL.video, topics: VIDEO_TOPICS },
  { id: "upscale", label: HELP_AREA_LABEL.upscale, topics: UPSCALE_TOPICS },
  { id: "voice", label: HELP_AREA_LABEL.voice, topics: VOICE_TOPICS },
  { id: "stories", label: HELP_AREA_LABEL.stories, topics: STORIES_TOPICS },
  { id: "dataset", label: HELP_AREA_LABEL.dataset, topics: [...DATASET_TOPICS, ...DATASET_CURATION_TOPICS] },
  { id: "training", label: HELP_AREA_LABEL.training, topics: TRAINING_TOPICS },
  { id: "jobs", label: HELP_AREA_LABEL.jobs, topics: JOBS_TOPICS },
  { id: "agents", label: HELP_AREA_LABEL.agents, topics: AGENTS_TOPICS },
  { id: "models", label: HELP_AREA_LABEL.models, topics: MODELS_TOPICS },
  { id: "benchmark", label: HELP_AREA_LABEL.benchmark, topics: BENCHMARK_TOPICS },
  { id: "diagnostics", label: HELP_AREA_LABEL.diagnostics, topics: DIAGNOSTICS_TOPICS },
  { id: "settings", label: HELP_AREA_LABEL.settings, topics: SETTINGS_TOPICS },
  { id: "shortcuts", label: HELP_AREA_LABEL.shortcuts, topics: SHORTCUTS_TOPICS },
];

/** Every topic across all areas, in Help-tab order (the palette lists these). */
export const HELP_TOPICS: readonly HelpTopic[] = HELP_AREAS.flatMap((a) => a.topics);

/** The one topic of `area` with this id, or `null`. */
export function findTopic(area: HelpArea, topicId: string): HelpTopic | null {
  return HELP_TOPICS.find((t) => t.area === area && t.id === topicId) ?? null;
}

/** The setting `key` of `area` and the topic that carries it, or `null`. */
export function findSetting(
  area: HelpArea,
  key: string,
): { topic: HelpTopic; setting: HelpSetting } | null {
  for (const topic of HELP_TOPICS) {
    if (topic.area !== area) continue;
    const setting = topic.settings?.find((s) => s.key === key);
    if (setting) return { topic, setting };
  }
  return null;
}

/** DOM ids for deep links. Deterministic (not `useId`): the Help tab is
 *  mounted once, and hints elsewhere need to name these without a ref. */
export const topicDomId = (topic: HelpTopic) => `help-${topic.area}-${topic.id}`;
export const settingDomId = (area: HelpArea, key: string) => `help-${area}-setting-${key}`;

/** One topic's share of a search result: whether the topic's own text
 *  matched, and which of its settings did. */
export interface HelpSearchHit {
  topic: HelpTopic;
  topicMatched: boolean;
  settings: readonly HelpSetting[];
}

const settingText = (s: HelpSetting) =>
  [s.label, s.what, s.why, s.effect, s.benefit, s.pitfalls ?? "", s.measured ?? ""].join("\n");

const topicText = (t: HelpTopic) =>
  [
    t.title,
    t.summary,
    ...t.body.map((b) => (b.kind === "p" ? b.text : b.items.join("\n"))),
  ].join("\n");

/** Case-insensitive substring search over titles, summaries, body text and
 *  every setting field. Returns the topics that matched (themselves or via a
 *  setting), in Help-tab order; `count` is topics + settings matched. */
export function searchHelp(query: string): { hits: HelpSearchHit[]; count: number } {
  const q = query.trim().toLowerCase();
  if (!q) return { hits: [], count: 0 };
  const hits: HelpSearchHit[] = [];
  let count = 0;
  for (const topic of HELP_TOPICS) {
    const topicMatched = topicText(topic).toLowerCase().includes(q);
    const settings = (topic.settings ?? []).filter((s) => settingText(s).toLowerCase().includes(q));
    if (!topicMatched && settings.length === 0) continue;
    hits.push({ topic, topicMatched, settings });
    count += (topicMatched ? 1 : 0) + settings.length;
  }
  return { hits, count };
}
