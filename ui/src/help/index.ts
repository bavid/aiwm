import { DATASET_CURATION_TOPICS } from "./dataset-curation.ts";
import { DATASET_TOPICS } from "./dataset.ts";
import { TRAINING_TOPICS } from "./training.ts";
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
  { id: "dataset", label: HELP_AREA_LABEL.dataset, topics: [...DATASET_TOPICS, ...DATASET_CURATION_TOPICS] },
  { id: "training", label: HELP_AREA_LABEL.training, topics: TRAINING_TOPICS },
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
