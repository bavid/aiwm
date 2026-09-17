import type { TrainingHyperparams } from "../../lib/ipc";

/** The four per-run overrides, as raw input strings. A blank field keeps the
 *  preset's own value, which is why these are strings and not numbers: `0` and
 *  "left alone" are different answers, and a `number` cannot hold both. */
export type TuneDraft = {
  rank: string;
  lr: string;
  resolution: string;
  steps: string;
};

export type TuneField = keyof TuneDraft;

/** Per-field complaints, keyed the same way the draft is. Empty when every
 *  field is either blank or valid. */
export type TuneErrors = Partial<Record<TuneField, string>>;

export type ParsedTune = {
  values: TrainingHyperparams;
  errors: TuneErrors;
};

/** Fields the trainer counts rather than measures — a LoRA rank of 15.5 or
 *  1200.7 training steps are not values ai-toolkit can act on, so they are
 *  rejected here instead of failing deep inside the run. */
const INTEGER_FIELDS: readonly TuneField[] = ["rank", "steps"];

function parseField(field: TuneField, raw: string): { value: number | null; error?: string } {
  const text = raw.trim();
  // Blank is the normal case, not an omission: it means "use the preset".
  if (text === "") return { value: null };

  const n = Number(text);
  if (!Number.isFinite(n)) return { value: null, error: "Enter a number." };
  if (n <= 0) return { value: null, error: "Enter a number above zero." };
  if (INTEGER_FIELDS.includes(field) && !Number.isInteger(n)) {
    return { value: null, error: "Enter a whole number." };
  }
  return { value: n };
}

/** Reads a fine-tuning draft into the wire shape, collecting every complaint
 *  rather than stopping at the first — the form shows them all at once. */
export function parseTune(draft: TuneDraft): ParsedTune {
  const errors: TuneErrors = {};
  const read = (field: TuneField) => {
    const { value, error } = parseField(field, draft[field]);
    if (error) errors[field] = error;
    return value;
  };
  // Read every field before returning, so `errors` is complete.
  const values: TrainingHyperparams = {
    steps: read("steps"),
    lr: read("lr"),
    rank: read("rank"),
    resolution: read("resolution"),
  };
  return { values, errors };
}
