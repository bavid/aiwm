import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import type { TrainingPresetValues } from "../../lib/ipc";
import type { TuneDraft, TuneErrors, TuneField } from "./tune";

/** The four fields' hints, spelled out so `scripts/check-help.mjs` can read
 *  the keys as literals. */
function FieldHint({ field, describes }: { field: TuneField; describes: string }) {
  switch (field) {
    case "rank":
      return <HelpHint area="training" setting="rank" describes={describes} />;
    case "lr":
      return <HelpHint area="training" setting="learning-rate" describes={describes} />;
    case "resolution":
      return <HelpHint area="training" setting="resolution" describes={describes} />;
    case "steps":
      return <HelpHint area="training" setting="steps" describes={describes} />;
  }
}

type Props = {
  value: TuneDraft;
  onChange: (next: TuneDraft) => void;
  /** Per-field complaints from `parseTune`; the owner also blocks Start. */
  errors: TuneErrors;
  /** The active preset's values — shown as placeholders, so an empty field
   *  still says what it would use. */
  preset: TrainingPresetValues | null;
  /** When continuing an existing LoRA its rank is not a choice: the trainer
   *  would silently pad or truncate the weights to any other. `null` leaves
   *  the field editable. */
  lockedRank: number | null;
};

const FIELDS: { field: TuneField; label: string; min: number; step?: number | "any" }[] = [
  { field: "rank", label: "Rank", min: 1, step: 1 },
  { field: "lr", label: "Learning rate", min: 0, step: "any" },
  { field: "resolution", label: "Resolution", min: 64, step: 64 },
  { field: "steps", label: "Steps", min: 1, step: 1 },
];

/** The collapsed "Fine-tune" section of the run form: rank, learning rate,
 *  resolution and steps. Collapsed by default — the presets are the intended
 *  way in, and these four are for someone who already knows what they change.
 *  Opens on its own while a rank is pinned, so the explanation is in view. */
export function FineTune({ value, onChange, errors, preset, lockedRank }: Props) {
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
  const isOpen = open || lockedRank !== null;

  return (
    <div>
      <button
        type="button"
        className="chip"
        aria-expanded={isOpen}
        aria-controls={regionId}
        onClick={() => setOpen((v) => !v)}
      >
        Fine-tune
      </button>
      <div className="runform__tune" id={regionId} hidden={!isOpen}>
        {FIELDS.map(({ field, label, min, step }) => {
          const error = errors[field];
          const errorId = `${fieldIds[field]}-error`;
          const noteId = `${fieldIds[field]}-note`;
          const isLocked = field === "rank" && lockedRank !== null;
          const describedBy = error ? errorId : isLocked ? noteId : undefined;
          return (
            <div className="datasetform__field" key={field}>
              <span>
                <label htmlFor={fieldIds[field]}>{label}</label>
                <FieldHint field={field} describes={fieldIds[field]} />
              </span>
              <input
                id={fieldIds[field]}
                type="number"
                min={min}
                step={step}
                value={isLocked ? String(lockedRank) : value[field]}
                disabled={isLocked}
                onChange={(e) => patch(field, e.target.value)}
                placeholder={preset ? String(preset[field]) : "preset"}
                aria-invalid={error ? true : undefined}
                aria-describedby={describedBy}
              />
              {error && (
                <em className="runform__fielderr" id={errorId}>
                  {error}
                </em>
              )}
              {isLocked && !error && (
                <em className="runform__fieldnote" id={noteId}>
                  Rank {lockedRank} — fixed by the LoRA you continue from
                </em>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
