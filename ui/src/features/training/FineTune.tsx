import { useId, useState } from "react";
import type { TrainingPresetValues } from "../../lib/ipc";
import type { TuneDraft, TuneErrors, TuneField } from "./tune";

type Props = {
  value: TuneDraft;
  onChange: (next: TuneDraft) => void;
  /** Per-field complaints from `parseTune`; the owner also blocks Start. */
  errors: TuneErrors;
  /** The active preset's values — shown as placeholders, so an empty field
   *  still says what it would use. */
  preset: TrainingPresetValues | null;
};

const FIELDS: { field: TuneField; label: string; min: number; step?: number | "any" }[] = [
  { field: "rank", label: "Rank", min: 1, step: 1 },
  { field: "lr", label: "Learning rate", min: 0, step: "any" },
  { field: "resolution", label: "Resolution", min: 64, step: 64 },
  { field: "steps", label: "Steps", min: 1, step: 1 },
];

/** The collapsed "Fine-tune" section of the run form: rank, learning rate,
 *  resolution and steps. Collapsed by default — the presets are the intended
 *  way in, and these four are for someone who already knows what they change. */
export function FineTune({ value, onChange, errors, preset }: Props) {
  const [open, setOpen] = useState(false);
  // One id per field plus one for the region the button discloses. `useId`
  // has to be called unconditionally, hence four fixed calls rather than a
  // loop over FIELDS.
  const regionId = useId();
  const fieldIds: Record<TuneField, string> = {
    rank: useId(),
    lr: useId(),
    resolution: useId(),
    steps: useId(),
  };

  const patch = (field: TuneField, next: string) => onChange({ ...value, [field]: next });

  return (
    <div>
      <button
        type="button"
        className="chip"
        aria-expanded={open}
        aria-controls={regionId}
        onClick={() => setOpen((v) => !v)}
      >
        Fine-tune
      </button>
      <div className="runform__tune" id={regionId} hidden={!open}>
        {FIELDS.map(({ field, label, min, step }) => {
          const error = errors[field];
          const errorId = `${fieldIds[field]}-error`;
          return (
            <label className="datasetform__field" htmlFor={fieldIds[field]} key={field}>
              <span>{label}</span>
              <input
                id={fieldIds[field]}
                type="number"
                min={min}
                step={step}
                value={value[field]}
                onChange={(e) => patch(field, e.target.value)}
                placeholder={preset ? String(preset[field]) : "preset"}
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? errorId : undefined}
              />
              {error && (
                <em className="runform__fielderr" id={errorId}>
                  {error}
                </em>
              )}
            </label>
          );
        })}
      </div>
    </div>
  );
}
