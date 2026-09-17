import { useId, useState } from "react";
import {
  HIRES_DENOISE_MAX,
  HIRES_DENOISE_MIN,
  HIRES_DENOISE_STEP,
  HIRES_SCALES,
  HIRES_UPSCALE_METHODS,
  clampHiresSteps,
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
                {/* The slider itself announces its value, so the visible
                    read-out would only churn the label's accessible name. */}
                Denoise{" "}
                <span className="hiresfix__value" aria-hidden="true">
                  {value.denoise.toFixed(2)}
                </span>
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
          {/* Unmounted, not hidden: a `hidden` panel's out-of-range number
              input still takes part in the form's constraint validation, and
              the browser then refuses to submit a control it cannot focus.
              The parent owns every value, so nothing is lost by unmounting. */}
          {advancedOpen && (
            <div className="hiresfix__adv" id={advancedId}>
              <label className="imgform__field">
                <span>Steps</span>
                <input
                  type="number"
                  inputMode="numeric"
                  step={1}
                  value={value.steps}
                  placeholder={String(hiresAutoSteps(steps))}
                  onChange={(e) => patch({ steps: e.target.value.replace(/[^\d]/g, "") })}
                  onBlur={(e) => {
                    const v = e.target.value;
                    patch({ steps: v === "" ? "" : String(clampHiresSteps(Number(v))) });
                  }}
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
          )}
        </>
      )}
    </fieldset>
  );
}
