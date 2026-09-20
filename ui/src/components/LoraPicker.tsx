import { useId } from "react";
import { type LoraParam, type Model } from "../lib/ipc";
import { HelpHint } from "./HelpHint";
import "./lora-picker.css";

const MIN_STRENGTH = 0;
const MAX_STRENGTH = 2;
/** Matches the strength `core::pipeline`'s own tests reach for
 *  (`add-detail-xl.safetensors @ 0.8`, `flux-2-realistic-detail @ 0.8`) —
 *  a sensible middle ground rather than ComfyUI's raw default of 1.0. */
const DEFAULT_STRENGTH = 0.8;

const clampStrength = (n: number) => Math.min(MAX_STRENGTH, Math.max(MIN_STRENGTH, n));

/** Checkpoint/model families whose ComfyUI pipeline has no LoRA seam at all —
 *  `core::pipeline` would build their graph without a `loras: &[LoraSpec]`
 *  parameter, so showing this picker would offer a control that silently
 *  does nothing. Empty today: every recipe AIWM ships (SDXL / plain
 *  checkpoint, FLUX.1, FLUX.2 [klein] GGUF/safetensors/edit, Wan 2.2,
 *  LTX-Video) threads a LoRA chain through `apply_loras`
 *  (`core/src/pipeline/mod.rs`). Kept as an explicit (currently empty) list
 *  rather than always returning `true` so a future recipe without the seam
 *  has an obvious place to be named instead of the picker quietly lying
 *  about support. */
const FAMILIES_WITHOUT_LORA_SEAM: readonly string[] = [];

function familySupportsLora(family: string | null | undefined): boolean {
  return !family || !FAMILIES_WITHOUT_LORA_SEAM.includes(family.toLowerCase());
}

/** Optional multi-select LoRA stack layered on top of the base model — 0 or
 *  more, each with its own strength. Mirrors the "Edit an existing image" /
 *  "start from an image" fieldsets' own optional-section convention: nothing
 *  selected by default, so a render is identical to not having this picker
 *  at all until the user opts in.
 *
 *  Hidden entirely (not just disabled) when `family` names a checkpoint
 *  family with no LoRA seam in `core::pipeline` — see
 *  `FAMILIES_WITHOUT_LORA_SEAM`. Otherwise it stays visible even with zero
 *  LoRAs imported yet, with guidance on where to get one, rather than
 *  disappearing (which would look like the feature vanished).
 *
 *  The help lives under the Image area (`lora-stack`, `lora-strength`); the
 *  Video tab shares this picker and that text. */
export function LoraPicker({
  models,
  family,
  selected,
  onChange,
}: {
  models: Model[];
  /** The base model's family (`"flux"`, `"sdxl"`, `"wan"`, …) — used both to
   *  decide whether the whole section applies (a family with no LoRA seam
   *  hides it) and to filter which LoRAs are offered (a LoRA with a
   *  different, known family isn't interchangeable; one with no recognizable
   *  family always shows, since it can't be ruled out). */
  family: string | null | undefined;
  selected: LoraParam[];
  onChange: (next: LoraParam[]) => void;
}) {
  const baseId = useId();
  if (!familySupportsLora(family)) return null;

  const loras = models.filter(
    (m) => m.roles.includes("lora") && (!family || !m.family || m.family === family),
  );

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
    <fieldset className="lora-picker" id={baseId}>
      <legend>
        LoRAs (optional) <HelpHint area="image" setting="lora-stack" describes={baseId} />
      </legend>
      {loras.length === 0 ? (
        <p className="muted">
          No LoRAs imported yet — import a <code>.safetensors</code> LoRA (role “LoRA”) on the
          Models tab to stack one onto this render.
        </p>
      ) : (
        loras.map((m) => {
          const active = entryFor(m.id);
          const strengthId = `${baseId}-strength-${m.id}`;
          return (
            <div key={m.id} className="lora-picker__item">
              <label className="lora-picker__row">
                <input type="checkbox" checked={!!active} onChange={() => toggle(m.id)} />
                <span className="lora-picker__name">
                  {m.name}
                  {active && (
                    <span className="lora-picker__value"> — {active.strength.toFixed(2)}</span>
                  )}
                </span>
              </label>
              {active && (
                <div className="lora-picker__strength-row">
                  <input
                    id={strengthId}
                    type="range"
                    className="lora-picker__strength"
                    aria-label={`Strength of ${m.name}`}
                    min={MIN_STRENGTH}
                    max={MAX_STRENGTH}
                    step={0.05}
                    value={active.strength}
                    onChange={(e) => {
                      const n = Number(e.target.value);
                      if (Number.isFinite(n)) setStrength(m.id, clampStrength(n));
                    }}
                  />
                  <HelpHint area="image" setting="lora-strength" describes={strengthId} />
                </div>
              )}
            </div>
          );
        })
      )}
    </fieldset>
  );
}
