import type { Model } from "../../lib/ipc";

/** The form's size / steps / CFG defaults per checkpoint family — the UI side
 *  of `core/src/capability/image_defaults.rs` (keep the two in step). SD 1.5
 *  was trained at 512 px: 512×512, 20 steps, CFG 8 are the values of
 *  ComfyUI v0.34.0's own SD 1.5 example; Krea 2 Turbo runs at 1024×1024,
 *  8 steps, CFG 1 (Comfy-Org's own Turbo template); everything else (SDXL,
 *  FLUX) starts at 1024×1024, 25 steps, CFG 7. */
export interface ImageDefaults {
  width: number;
  height: number;
  steps: number;
  cfg: number;
}

export const STANDARD_DEFAULTS: ImageDefaults = { width: 1024, height: 1024, steps: 25, cfg: 7 };
export const SD15_DEFAULTS: ImageDefaults = { width: 512, height: 512, steps: 20, cfg: 8 };
export const KREA2_DEFAULTS: ImageDefaults = { width: 1024, height: 1024, steps: 8, cfg: 1 };

type FamilyFields = Pick<Model, "base_family" | "family"> & Partial<Pick<Model, "name" | "file_path">>;

const recordedFamily = (m: FamilyFields | null | undefined) =>
  (m?.base_family ?? m?.family ?? "").trim().toLowerCase().replaceAll("_", "-");

/** Whether a checkpoint is SD 1.5: its recorded base family, else its
 *  legacy family (a catalog SD 1.5 import records `family = "sd15"`). */
export function isSd15(m: FamilyFields | null | undefined): boolean {
  return recordedFamily(m) === "sd15";
}

/** `krea2` / `Krea 2` / `krea-2` in a name that is not a FLUX name — the
 *  same rule as `family_from_name` in `core/src/model/family/detect.rs`
 *  (FLUX.1 Krea [dev] files are FLUX.1). */
const namesKrea2 = (s: string | undefined) => {
  const n = (s ?? "").toLowerCase().replace(/[^a-z0-9]/g, "");
  return n.includes("krea2") && !n.includes("flux");
};

/** Whether a checkpoint is Krea 2: its recorded family, else — when nothing
 *  is recorded (a Civitai import) — its name or file name. */
export function isKrea2(m: FamilyFields | null | undefined): boolean {
  const f = recordedFamily(m);
  if (f) return f === "krea2";
  return namesKrea2(m?.name) || namesKrea2(m?.file_path?.split(/[\\/]/).pop());
}

export const defaultsFor = (m: FamilyFields | null | undefined) =>
  isSd15(m) ? SD15_DEFAULTS : isKrea2(m) ? KREA2_DEFAULTS : STANDARD_DEFAULTS;

/** The value a field takes when the checkpoint's defaults change from `from`
 *  to `to`: the new default if the field still holds the old one (the user
 *  has not changed it), else what the user set. */
export const carryOver = (value: number, from: number, to: number) =>
  value === from ? to : value;
