import type { Model } from "../../lib/ipc";

/** The form's size / steps / CFG defaults per checkpoint family — the UI side
 *  of `core/src/capability/image_defaults.rs` (keep the two in step). SD 1.5
 *  was trained at 512 px: 512×512, 20 steps, CFG 8 are the values of
 *  ComfyUI v0.34.0's own SD 1.5 example; everything else (SDXL, FLUX) starts
 *  at 1024×1024, 25 steps, CFG 7. */
export interface ImageDefaults {
  width: number;
  height: number;
  steps: number;
  cfg: number;
}

export const STANDARD_DEFAULTS: ImageDefaults = { width: 1024, height: 1024, steps: 25, cfg: 7 };
export const SD15_DEFAULTS: ImageDefaults = { width: 512, height: 512, steps: 20, cfg: 8 };

/** Whether a checkpoint is SD 1.5: its recorded base family, else its
 *  legacy family (a catalog SD 1.5 import records `family = "sd15"`). */
export function isSd15(m: Pick<Model, "base_family" | "family"> | null | undefined): boolean {
  const f = (m?.base_family ?? m?.family ?? "").trim().toLowerCase().replaceAll("_", "-");
  return f === "sd15";
}

export const defaultsFor = (m: Pick<Model, "base_family" | "family"> | null | undefined) =>
  isSd15(m) ? SD15_DEFAULTS : STANDARD_DEFAULTS;

/** The value a field takes when the checkpoint's defaults change from `from`
 *  to `to`: the new default if the field still holds the old one (the user
 *  has not changed it), else what the user set. */
export const carryOver = (value: number, from: number, to: number) =>
  value === from ? to : value;
