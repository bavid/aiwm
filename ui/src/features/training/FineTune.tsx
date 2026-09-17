import { useId, useState } from "react";
import type { TrainingPresetValues } from "../../lib/ipc";

/** The four per-run overrides, as raw input strings. A blank field keeps the
 *  preset's own value, which is why these are strings and not numbers. */
export type TuneDraft = {
  rank: string;
  lr: string;
  resolution: string;
  steps: string;
};

type Props = {
  value: TuneDraft;
  onChange: (next: TuneDraft) => void;
  /** The active preset's values — shown as placeholders, so an empty field
   *  still says what it would use. */
  preset: TrainingPresetValues | null;
};

/** The collapsed "Fine-tune" section of the run form: rank, learning rate,
 *  resolution and steps. Collapsed by default — the presets are the intended
 *  way in, and these four are for someone who already knows what they change. */
export function FineTune({ value, onChange, preset }: Props) {
  const [open, setOpen] = useState(false);
  const ids = {
    rank: useId(),
    lr: useId(),
    resolution: useId(),
    steps: useId(),
  };

  const hint = (of: keyof TrainingPresetValues) => (preset ? String(preset[of]) : "preset");
  const patch = (field: keyof TuneDraft, next: string) => onChange({ ...value, [field]: next });

  return (
    <div>
      <button
        type="button"
        className="chip"
        aria-pressed={open}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        Fine-tune
      </button>
      <div className="runform__tune" hidden={!open}>
        <label className="datasetform__field" htmlFor={ids.rank}>
          <span>Rank</span>
          <input
            id={ids.rank}
            type="number"
            min={1}
            value={value.rank}
            onChange={(e) => patch("rank", e.target.value)}
            placeholder={hint("rank")}
          />
        </label>
        <label className="datasetform__field" htmlFor={ids.lr}>
          <span>Learning rate</span>
          <input
            id={ids.lr}
            type="number"
            step="any"
            min={0}
            value={value.lr}
            onChange={(e) => patch("lr", e.target.value)}
            placeholder={hint("lr")}
          />
        </label>
        <label className="datasetform__field" htmlFor={ids.resolution}>
          <span>Resolution</span>
          <input
            id={ids.resolution}
            type="number"
            min={64}
            step={64}
            value={value.resolution}
            onChange={(e) => patch("resolution", e.target.value)}
            placeholder={hint("resolution")}
          />
        </label>
        <label className="datasetform__field" htmlFor={ids.steps}>
          <span>Steps</span>
          <input
            id={ids.steps}
            type="number"
            min={1}
            value={value.steps}
            onChange={(e) => patch("steps", e.target.value)}
            placeholder={hint("steps")}
          />
        </label>
      </div>
    </div>
  );
}
