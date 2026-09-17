import { useId, useState } from "react";
import {
  HIRES_DENOISE_MAX,
  HIRES_DENOISE_MIN,
  HIRES_DENOISE_STEP,
  HIRES_SCALES,
  HIRES_STEPS_MAX,
  HIRES_STEPS_MIN,
  HIRES_UPSCALE_METHODS,
  hiresAutoSteps,
  hiresFinalDim,
  hiresWorkFactor,
  snapDenoise,
  type HiresFixSettings,
} from "./hires-fix";

type Props = {
  value: HiresFixSettings;
  onChange: (next: HiresFixSettings) => void;
  /** The first pass's size and step count — they drive the hint and the
   *  Steps placeholder. Already clamped by the caller. */
  width: number;
  height: number;
  steps: number;
};

/** Hi-Res-Fix: render once at the requested size, then upscale that latent
 *  and re-sample it at a low denoise. Text-to-image only — an image *edit*
 *  and Story Studio's own submit path never show this. */
export function HiresFixField({ value, onChange, width, height, steps }: Props) {
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const advancedId = useId();

  const patch = (next: Partial<HiresFixSettings>) => onChange({ ...value, ...next });

  const finalWidth = hiresFinalDim(width, value.scaleBy);
  const finalHeight = hiresFinalDim(height, value.scaleBy);

  return (
    <fieldset className="hiresfix">
      <legend>Hi-res fix</legend>
      <label className="hiresfix__check">
        <input
          type="checkbox"
          checked={value.enabled}
          onChange={(e) => patch({ enabled: e.target.checked })}
        />
        <span>Render once, then refine the result at a higher resolution</span>
      </label>

      {value.enabled && (
        <>
          <div className="imgform__grid">
            <label className="imgform__field">
              <span>Scale</span>
              <select
                value={String(value.scaleBy)}
                onChange={(e) => patch({ scaleBy: Number(e.target.value) })}
              >
                {HIRES_SCALES.map((s) => (
                  <option key={s} value={s}>
                    {s}×
                  </option>
                ))}
              </select>
            </label>
            <label className="imgform__field">
              <span>
                Denoise <span className="hiresfix__value">{value.denoise.toFixed(2)}</span>
              </span>
              <input
                type="range"
                className="hiresfix__slider"
                min={HIRES_DENOISE_MIN}
                max={HIRES_DENOISE_MAX}
                step={HIRES_DENOISE_STEP}
                value={value.denoise}
                onChange={(e) => {
                  const n = Number(e.target.value);
                  if (Number.isFinite(n)) patch({ denoise: snapDenoise(n) });
                }}
              />
            </label>
          </div>

          <p className="hiresfix__hint">
            Renders at {width}×{height} first, then refines at {finalWidth}×{finalHeight} — roughly{" "}
            {hiresWorkFactor(value.scaleBy)}× the sampling work.
          </p>

          <button
            type="button"
            className="chip hiresfix__advbtn"
            aria-expanded={advancedOpen}
            aria-controls={advancedId}
            onClick={() => setAdvancedOpen((v) => !v)}
          >
            Advanced
          </button>
          <div className="hiresfix__adv" id={advancedId} hidden={!advancedOpen}>
            <label className="imgform__field">
              <span>Steps</span>
              <input
                type="number"
                min={HIRES_STEPS_MIN}
                max={HIRES_STEPS_MAX}
                step={1}
                value={value.steps}
                placeholder={String(hiresAutoSteps(steps))}
                onChange={(e) => patch({ steps: e.target.value.replace(/[^\d]/g, "") })}
              />
            </label>
            <label className="imgform__field">
              <span>Upscale method</span>
              <select
                value={value.upscaleMethod}
                onChange={(e) => patch({ upscaleMethod: e.target.value })}
              >
                {HIRES_UPSCALE_METHODS.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </>
      )}
    </fieldset>
  );
}
