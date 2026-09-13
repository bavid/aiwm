import { useEffect, useState } from "react";

export type PresetKind = "positive" | "negative";

export interface PromptPreset {
  id: string;
  name: string;
  text: string;
}

const STORAGE_KEY: Record<PresetKind, string> = {
  positive: "aiwm.presets.positive",
  negative: "aiwm.presets.negative",
};

/** Seeded once per kind on first use -- after that, entirely the user's own
 *  list (rename/edit/delete all apply, including to these starting ones). */
const STARTER_PRESETS: Record<PresetKind, PromptPreset[]> = {
  positive: [
    { id: "starter-quality", name: "High detail", text: "highly detailed, sharp focus, intricate detail, high quality" },
    { id: "starter-cinematic", name: "Cinematic", text: "cinematic lighting, dramatic shadows, film grain, shot on 35mm" },
    { id: "starter-photo", name: "Realistic photo", text: "natural window light, soft shadows, candid framing, shot on iphone" },
  ],
  negative: [
    { id: "starter-basic", name: "Basic quality", text: "blurry, low quality, low resolution, watermark, text, signature" },
    { id: "starter-anatomy", name: "Bad anatomy", text: "extra limbs, extra fingers, deformed hands, mutated, disfigured" },
    { id: "starter-artifacts", name: "Artifacts", text: "jpeg artifacts, oversaturated, overexposed, noisy, grainy" },
  ],
};

function readPresets(kind: PresetKind): PromptPreset[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY[kind]);
    if (!raw) return STARTER_PRESETS[kind];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : STARTER_PRESETS[kind];
  } catch {
    return STARTER_PRESETS[kind];
  }
}

function writePresets(kind: PresetKind, presets: PromptPreset[]) {
  try {
    window.localStorage.setItem(STORAGE_KEY[kind], JSON.stringify(presets));
  } catch {
    /* localStorage unavailable -- presets just won't persist this session */
  }
}

let seq = 0;
const newId = () => `preset-${Date.now()}-${seq++}`;

/** A user's editable library of positive or negative prompt snippets,
 *  persisted to localStorage (per-browser, not synced) and seeded with a
 *  small curated starter set on first use. */
export function usePromptPresets(kind: PresetKind) {
  const [presets, setPresets] = useState<PromptPreset[]>(() => readPresets(kind));

  useEffect(() => {
    setPresets(readPresets(kind));
  }, [kind]);

  const commit = (next: PromptPreset[]) => {
    setPresets(next);
    writePresets(kind, next);
  };

  const add = (name: string, text: string) => {
    commit([...presets, { id: newId(), name, text }]);
  };

  const update = (id: string, name: string, text: string) => {
    commit(presets.map((p) => (p.id === id ? { ...p, name, text } : p)));
  };

  const remove = (id: string) => {
    commit(presets.filter((p) => p.id !== id));
  };

  return { presets, add, update, remove };
}
