import type { StartRunBody, TrainingHyperparams, TrainingPresetValues } from "../../lib/ipc";

/** The four numbers a run actually trains with, override or preset. `null`
 *  where neither side knows (no profile loaded yet, or an old row). */
export type EffectiveHyperparams = {
  rank: number | null;
  lr: number | null;
  resolution: number | null;
  steps: number | null;
};

/** The stored `hyperparams_json` of a run, parsed; `null` when it is not the
 *  object the form wrote (a hand-edited store, a future field shape). */
export function parseHyperparamsJson(json: string): TrainingHyperparams | null {
  try {
    const parsed: unknown = JSON.parse(json);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    return parsed as TrainingHyperparams;
  } catch {
    return null;
  }
}

/** A run's overrides laid over its preset — what the trainer was told. An
 *  override of `null`/absent falls through to the preset, as the core does. */
export function effectiveHyperparams(
  overrides: TrainingHyperparams | null,
  preset: TrainingPresetValues | null,
): EffectiveHyperparams {
  const pick = (field: keyof EffectiveHyperparams): number | null =>
    overrides?.[field] ?? preset?.[field] ?? null;
  return {
    rank: pick("rank"),
    lr: pick("lr"),
    resolution: pick("resolution"),
    steps: pick("steps"),
  };
}

/** Adds `init_lora_model_id` only when the form has a LoRA to continue —
 *  the wire field is optional, and an empty string would be refused. */
export function withInitLora<T extends object>(
  body: T,
  initLoraModelId: string | null,
): T & Pick<StartRunBody, "init_lora_model_id"> {
  return initLoraModelId ? { ...body, init_lora_model_id: initLoraModelId } : body;
}
