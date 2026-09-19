/** UI copy for the "Training & captioning" catalog stacks (`media:
 *  "training"`, `core::model::catalog`), keyed by stack id: what each tool is
 *  *for* and where it runs. The stack's own `note` carries the file/licence
 *  detail; this is the one-line "why would I pick this" the card leads with. */
export interface TrainingToolInfo {
  /** Short name for buttons and announcements. */
  shortName: string;
  purpose: string;
  /** CPU or GPU, with the VRAM it needs when it is the GPU. */
  runsOn: string;
  /** The Dataset tab captioner this stack installs; `null` for the
   *  escalation model, which is not a captioner of its own. */
  captionerId: string | null;
}

export const RECOMMENDED_CAPTIONER_STACK = "wd-tagger";

export const TRAINING_TOOLS: Readonly<Record<string, TrainingToolInfo>> = {
  "wd-tagger": {
    shortName: "WD tagger",
    purpose:
      "Tags: a Danbooru-style tag list per frame. Best for anime and illustration styles or characters.",
    runsOn: "CPU",
    captionerId: "wd-eva02-tagger-v3",
  },
  "florence2-large": {
    shortName: "Florence-2",
    purpose: "Prose: one descriptive sentence per frame. Best for photos and general subjects.",
    runsOn: "GPU · ~2 GiB VRAM",
    captionerId: "florence2",
  },
  "qwen2.5-vl-7b": {
    shortName: "Qwen2.5-VL",
    purpose:
      "Second opinion: re-captions frames where Florence-2 sounds unsure, comparing each with a later frame.",
    runsOn: "GPU · ~6 GiB VRAM (4-bit)",
    captionerId: null,
  },
};

/** The stack that installs a given Dataset-tab captioner. */
export function stackIdForCaptioner(captionerId: string): string | null {
  const hit = Object.entries(TRAINING_TOOLS).find(([, t]) => t.captionerId === captionerId);
  return hit ? hit[0] : null;
}
