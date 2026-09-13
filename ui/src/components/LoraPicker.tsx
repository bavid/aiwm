import { type LoraParam, type Model } from "../lib/ipc";
import "./lora-picker.css";

const MIN_STRENGTH = -2;
const MAX_STRENGTH = 2;
const DEFAULT_STRENGTH = 1;

const clampStrength = (n: number) => Math.min(MAX_STRENGTH, Math.max(MIN_STRENGTH, n));

/** Optional multi-select of LoRAs layered on top of the base model — 0 or
 *  more, each with its own strength. Mirrors the Video tab's "optional start
 *  frame" pattern: nothing selected by default, so a render is identical to
 *  not having this picker at all until the user opts in. Renders nothing when
 *  the library has no LoRA imported for this family yet. */
export function LoraPicker({
  models,
  family,
  selected,
  onChange,
}: {
  models: Model[];
  /** The base model's family (`"flux"`, `"sdxl"`, `"wan"`, …) — a LoRA with a
   *  different, known family is hidden (they aren't interchangeable). A LoRA
   *  with no recognizable family always shows, since it can't be ruled out. */
  family: string | null | undefined;
  selected: LoraParam[];
  onChange: (next: LoraParam[]) => void;
}) {
  const loras = models.filter(
    (m) => m.roles.includes("lora") && (!family || !m.family || m.family === family),
  );
  if (loras.length === 0) return null;

  const entryFor = (id: string) => selected.find((l) => l.model_id === id);

  const toggle = (id: string) => {
    onChange(
      entryFor(id)
        ? selected.filter((l) => l.model_id !== id)
        : [...selected, { model_id: id, strength: DEFAULT_STRENGTH }],
    );
  };

  const setStrength = (id: string, strength: number) => {
    onChange(selected.map((l) => (l.model_id === id ? { ...l, strength } : l)));
  };

  return (
    <fieldset className="lora-picker">
      <legend>LoRAs (optional)</legend>
      {loras.map((m) => {
        const active = entryFor(m.id);
        return (
          <label key={m.id} className="lora-picker__row">
            <input type="checkbox" checked={!!active} onChange={() => toggle(m.id)} />
            <span className="lora-picker__name">{m.name}</span>
            {active && (
              <input
                type="number"
                className="lora-picker__strength"
                value={active.strength}
                step={0.05}
                min={MIN_STRENGTH}
                max={MAX_STRENGTH}
                onChange={(e) => {
                  const n = Number(e.target.value);
                  if (Number.isFinite(n)) setStrength(m.id, clampStrength(n));
                }}
              />
            )}
          </label>
        );
      })}
    </fieldset>
  );
}
