import type { HiresFixParams } from "../../lib/ipc";

/** How much bigger the second pass may run. The engine clamps `scale_by` to
 *  1.25–2; the form offers the two useful stops. */
export const HIRES_SCALES = [1.5, 2] as const;
export const HIRES_DENOISE_MIN = 0.2;
export const HIRES_DENOISE_MAX = 0.7;
export const HIRES_DENOISE_STEP = 0.05;
export const HIRES_STEPS_MIN = 4;
export const HIRES_STEPS_MAX = 60;
export const HIRES_UPSCALE_METHODS = [
  "nearest-exact",
  "bilinear",
  "area",
  "bicubic",
  "bislerp",
] as const;

/** One latent unit is 8 px — `LatentUpscaleBy` rounds in latent units. */
const LATENT_PX = 8;

/** The Hi-Res-Fix form state. `steps` stays a string so "blank" (= half the
 *  first pass, resolved by the engine) is a real value and not a magic 0. */
export type HiresFixSettings = {
  enabled: boolean;
  scaleBy: number;
  denoise: number;
  steps: string;
  upscaleMethod: string;
};

export const DEFAULT_HIRES: HiresFixSettings = {
  enabled: false,
  scaleBy: 1.5,
  denoise: 0.45,
  steps: "",
  upscaleMethod: "nearest-exact",
};

const clamp = (n: number, min: number, max: number) => Math.min(max, Math.max(min, n));

/** Snap to the slider's own grid so 0.2 + 8 × 0.05 stays `0.6`, not
 *  `0.6000000000000001`, in the submitted params. */
export function snapDenoise(n: number): number {
  const stepped = Math.round(n / HIRES_DENOISE_STEP) * HIRES_DENOISE_STEP;
  return Number(clamp(stepped, HIRES_DENOISE_MIN, HIRES_DENOISE_MAX).toFixed(2));
}

/** The engine's own 4–60 clamp on the second pass's step count. The form
 *  applies it on blur so a typed-out-of-range value is corrected in place
 *  rather than only on submit. */
export function clampHiresSteps(n: number): number {
  return clamp(Math.floor(n), HIRES_STEPS_MIN, HIRES_STEPS_MAX);
}

/** What the engine pins back when `steps` is left blank — half the first
 *  pass, inside the same 4–60 clamp (`ImageRequest`'s Hi-Res-Fix parsing). */
export function hiresAutoSteps(firstPassSteps: number): number {
  return clamp(Math.floor(firstPassSteps / 2), HIRES_STEPS_MIN, HIRES_STEPS_MAX);
}

/** `LatentUpscaleBy` rounds in latent units, so the decoded size is
 *  `round(px / 8 × scale) × 8` — the same arithmetic as
 *  `ImageRequest::final_size`. */
export function hiresFinalDim(px: number, scaleBy: number): number {
  return Math.round(Math.trunc(px / LATENT_PX) * scaleBy) * LATENT_PX;
}

/** The second pass resamples a `scale²`-bigger latent, so the sampling work
 *  grows with the square of the scale. Trailing zeros are trimmed — "4×",
 *  not "4.00×". */
export function hiresWorkFactor(scaleBy: number): string {
  return (scaleBy * scaleBy).toFixed(2).replace(/\.?0+$/, "");
}

/** The wire shape of the current settings. Only called when the toggle is on. */
export function toHiresParams(value: HiresFixSettings): HiresFixParams {
  const params: HiresFixParams = {
    scale_by: value.scaleBy,
    denoise: value.denoise,
    upscale_method: value.upscaleMethod,
  };
  const n = Number(value.steps);
  if (value.steps.trim() !== "" && Number.isFinite(n)) {
    params.steps = clampHiresSteps(n);
  }
  return params;
}
