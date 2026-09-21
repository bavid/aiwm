import type { Model } from "./ipc";

/** The base families AIWM knows, with their architecture group — a copy of
 *  `FAMILIES` in `core/src/model/family.rs`. It must stay in sync with that
 *  registry (ids, labels, arch groups, runnable).
 *
 *  Why a client-side copy: the backend exposes a family's `arch_group` only on
 *  the packages API (`FamilyRef`), not on `Model`, and the LoRA pickers and
 *  the "What base is this for?" select need it for every model without a
 *  request per render. Lives in `lib/` (not next to
 *  `features/models/package-plan.ts`'s not-runnable copy) because
 *  `components/LoraPicker.tsx` uses it and components never import from
 *  features. A stale copy can only mis-filter the picker (an unknown id
 *  always shows) or omit a choice from the select; it never writes a wrong
 *  family on its own. */
export interface KnownBaseFamily {
  id: string;
  label: string;
  /** Families in one group load each other's LoRAs (Pony works with SDXL). */
  archGroup: string;
  /** Why it cannot run here; `null` when it can. */
  notRunnable: string | null;
}

export const BASE_FAMILIES: readonly KnownBaseFamily[] = [
  { id: "sd15", label: "Stable Diffusion 1.5", archGroup: "sd15", notRunnable: null },
  { id: "sdxl", label: "Stable Diffusion XL", archGroup: "sdxl", notRunnable: null },
  { id: "pony", label: "Pony Diffusion (SDXL)", archGroup: "sdxl", notRunnable: null },
  { id: "illustrious", label: "Illustrious (SDXL)", archGroup: "sdxl", notRunnable: null },
  { id: "noobai", label: "NoobAI (SDXL)", archGroup: "sdxl", notRunnable: null },
  { id: "flux1", label: "FLUX.1 [dev]", archGroup: "flux1", notRunnable: null },
  { id: "flux1-schnell", label: "FLUX.1 [schnell]", archGroup: "flux1", notRunnable: null },
  { id: "flux1-krea", label: "FLUX.1 Krea [dev]", archGroup: "flux1", notRunnable: null },
  { id: "flux2-klein-4b", label: "FLUX.2 [klein] 4B", archGroup: "flux2-klein-4b", notRunnable: null },
  { id: "flux2-klein-9b", label: "FLUX.2 [klein] 9B", archGroup: "flux2-klein-9b", notRunnable: null },
  { id: "wan22-5b", label: "Wan 2.2 TI2V-5B", archGroup: "wan22-5b", notRunnable: null },
  {
    id: "wan-14b",
    label: "Wan 14B",
    archGroup: "wan-14b",
    notRunnable: "does not fit 16 GB — this app runs the 5B",
  },
  { id: "ltxv", label: "LTX-Video (2B)", archGroup: "ltxv", notRunnable: null },
  {
    id: "ltx2",
    label: "LTX-2",
    archGroup: "ltx2",
    notRunnable:
      "LTX-2 is a different, much larger audio-video model — this app runs LTX-Video 0.9.5 (2B)",
  },
];

export const baseFamilyById = (id: string | null | undefined): KnownBaseFamily | null =>
  id ? (BASE_FAMILIES.find((f) => f.id === id.trim().toLowerCase().replaceAll("_", "-")) ?? null) : null;

/** A model's registry family: its recorded `base_family`, else a legacy
 *  `family` string that already is a registry id (`sdxl`, `sd15`). The
 *  ambiguous legacy strings (`flux`, `flux2`, `wan`, `ltx`) stay unmapped —
 *  `flux2` could be the 4B or the 9B, whose LoRAs do not mix. */
export function registryFamilyOf(m: Pick<Model, "base_family" | "family">): KnownBaseFamily | null {
  return baseFamilyById(m.base_family) ?? baseFamilyById(m.family);
}

/** Whether `lora` can be stacked on `base`. Both registry families known:
 *  same architecture group (a Pony LoRA on an SDXL checkpoint). Otherwise
 *  the legacy family strings, as before Plan 14. Anything unknown on
 *  either side shows, since it can't be ruled out. */
export function loraFitsBase(
  lora: Pick<Model, "base_family" | "family">,
  base: Pick<Model, "base_family" | "family"> | null | undefined,
): boolean {
  if (!base) return true;
  const lf = registryFamilyOf(lora);
  const bf = registryFamilyOf(base);
  if (lf && bf) return lf.archGroup === bf.archGroup;
  return !base.family || !lora.family || lora.family === base.family;
}
