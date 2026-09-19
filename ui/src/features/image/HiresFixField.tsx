import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
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
 *  and Story Studio's own submit path never show this. Every caption is a
 *  `<label for>` beside its `?` hint, never wrapping it, so the hint's name
 *  never joins the control's. */
export function HiresFixField({ value, onChange, width, height, steps }: Props) {
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const ids = useId();
  const advancedId = `${ids}-adv`;
  const enableId = `${ids}-enable`;
  const scaleId = `${ids}-scale`;
  const denoiseId = `${ids}-denoise`;
  const stepsId = `${ids}-steps`;
  const methodId = `${ids}-method`;

  const patch = (next: Partial<HiresFixSettings>) => onChange({ ...value, ...next });

  const finalWidth = hiresFinalDim(width, value.scaleBy);
  const finalHeight = hiresFinalDim(height, value.scaleBy);

  return (
    <fieldset className="hiresfix">
      <legend>Hi-res fix</legend>
      <div className="hiresfix__check">
        <input
          id={enableId}
          type="checkbox"
          checked={value.enabled}
          onChange={(e) => patch({ enabled: e.target.checked })}
        />
        <label htmlFor={enableId}>Render once, then refine the result at a higher resolution</label>
        <HelpHint area="image" setting="hires-enable" describes={enableId} />
      </div>

      {value.enabled && (
        <>
          <div className="imgform__grid">
            <div className="imgform__field">
              <span>
                <label htmlFor={scaleId}>Scale</label>
                <HelpHint area="image" setting="hires-scale" describes={scaleId} />
              </span>
              <select
                id={scaleId}
                value={String(value.scaleBy)}
                onChange={(e) => patch({ scaleBy: Number(e.target.value) })}
              >
                {HIRES_SCALES.map((s) => (
                  <option key={s} value={s}>
                    {s}×
                  </option>
                ))}
              </select>
            </div>
            <div className="imgform__field">
              <span>
                <label htmlFor={denoiseId}>Denoise</label>
                {/* The slider itself announces its value, so the visible
                    read-out stays out of the label. */}
                <span className="hiresfix__value" aria-hidden="true">
                  {value.denoise.toFixed(2)}
                </span>
                <HelpHint area="image" setting="hires-denoise" describes={denoiseId} />
              </span>
              <input
                id={denoiseId}
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
            </div>
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
              <div className="imgform__field">
                <span>
                  <label htmlFor={stepsId}>Steps</label>
                  <HelpHint area="image" setting="hires-steps" describes={stepsId} />
                </span>
                <input
                  id={stepsId}
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
              </div>
              <div className="imgform__field">
                <span>
                  <label htmlFor={methodId}>Upscale method</label>
                  <HelpHint area="image" setting="hires-upscale-method" describes={methodId} />
                </span>
                <select
                  id={methodId}
                  value={value.upscaleMethod}
                  onChange={(e) => patch({ upscaleMethod: e.target.value })}
                >
                  {HIRES_UPSCALE_METHODS.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          )}
        </>
      )}
    </fieldset>
  );
}
