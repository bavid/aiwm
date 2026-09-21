/** Dev-mock for the Plan 14 package commands (`resolve_package`,
 *  `library_packages`, `set_model_base_family`, `save_base_families`). A
 *  small stand-in for `core::model::packages` over the dev-mock library:
 *  an SDXL group that is complete (two checkpoints), an Illustrious
 *  checkpoint, a FLUX.2 [klein] 9B group missing its text encoder, a klein 4B
 *  LoRA whose base is only findable (plus the trainer's 4B folder), two Pony
 *  LoRAs with a findable Pony checkpoint, a Wan 14B LoRA that cannot run
 *  here, and Krea 2 checkpoints / LoRAs of unknown base. Some rows carry no recorded
 *  `base_family` but a family "detected on read" (`DETECTED`, standing in
 *  for the header / file-name inference), which is what "Save detected
 *  families" persists. Not a test double for logic. */

import { BASE_FAMILIES } from "./base-families";

type AnyRecord = Record<string, unknown>;

interface MockFamily {
  id: string;
  label: string;
  arch_group: string;
  runnable: AnyRecord;
  /** Catalog base: [known id, name, size]. */
  base?: [string, string, number];
  /** Catalog companions: [role, known id, name, size, the mock model that has it]. */
  companions: [string, string, string, number, string | null][];
  civitai_label: string;
}

const WAN_14B_REASON = "does not fit 16 GB — this app runs the 5B";

const FAMILIES: MockFamily[] = [
  { id: "sd15", label: "Stable Diffusion 1.5", arch_group: "sd15", runnable: { kind: "yes" },
    base: ["sd15-v1-5-emaonly", "Stable Diffusion 1.5 (pruned, EMA-only)", 4_265_146_304], companions: [], civitai_label: "SD 1.5" },
  { id: "sdxl", label: "Stable Diffusion XL", arch_group: "sdxl", runnable: { kind: "yes" },
    base: ["sdxl-base-1.0", "Stable Diffusion XL 1.0 (base)", 6_938_078_334], companions: [], civitai_label: "SDXL 1.0" },
  { id: "pony", label: "Pony Diffusion (SDXL)", arch_group: "sdxl", runnable: { kind: "yes" },
    companions: [], civitai_label: "Pony" },
  { id: "illustrious", label: "Illustrious (SDXL)", arch_group: "sdxl", runnable: { kind: "yes" },
    companions: [], civitai_label: "Illustrious" },
  { id: "noobai", label: "NoobAI (SDXL)", arch_group: "sdxl", runnable: { kind: "yes" },
    companions: [], civitai_label: "NoobAI" },
  { id: "flux1", label: "FLUX.1 [dev]", arch_group: "flux1", runnable: { kind: "yes" },
    base: ["flux1-dev-q8", "FLUX.1-dev — Q8_0 (GGUF)", 12_708_281_504],
    companions: [
      ["text_encoder", "t5xxl-fp8", "T5-XXL — fp8 (Flux text encoder)", 4_893_934_904, null],
      ["text_encoder", "clip-l", "CLIP-L (Flux text encoder)", 246_144_152, null],
      ["vae", "flux-vae", "FLUX.1 VAE (ae)", 335_304_388, null],
    ], civitai_label: "Flux.1 D" },
  { id: "flux2-klein-9b", label: "FLUX.2 [klein] 9B", arch_group: "flux2-klein-9b", runnable: { kind: "yes" },
    base: ["flux2-klein-9b-q4", "FLUX.2 [klein] 9B — Q4_K_M (GGUF)", 5_909_829_920],
    companions: [
      ["text_encoder", "qwen3-8b-flux2-encoder", "Qwen3-8B — fp8 mixed (FLUX.2 text encoder)", 8_664_848_742, null],
      ["vae", "flux2-vae", "FLUX.2 VAE", 336_211_292, "m-flux2-vae"],
      ["vae", "flux2-klein-edit-vae", "FLUX.2 Edit VAE (small decoder)", 249_519_092, "m-flux2-edit-vae"],
    ], civitai_label: "Flux.2 Klein 9B" },
  { id: "wan22-5b", label: "Wan 2.2 TI2V-5B", arch_group: "wan22-5b", runnable: { kind: "yes" },
    base: ["wan22-ti2v-5b", "Wan 2.2 TI2V-5B — fp16 (video, default)", 9_999_658_848],
    companions: [
      ["text_encoder", "wan-umt5-xxl-fp8", "umt5-XXL — fp8 (Wan text encoder)", 6_735_906_897, "m-umt5"],
      ["vae", "wan22-vae", "Wan 2.2 VAE", 1_409_400_960, "m-wanvae"],
    ], civitai_label: "Wan Video 2.2 TI2V-5B" },
  { id: "wan-14b", label: "Wan 14B", arch_group: "wan-14b", runnable: { kind: "no", reason: WAN_14B_REASON },
    companions: [], civitai_label: "Wan Video 2.2 I2V-A14B" },
];

/** The top Pony checkpoints as Civitai answered `baseModels=Pony` on 2026-09-21. */
const PONY_CANDIDATES: AnyRecord[] = [
  { model_id: "257749", version_id: "290640", name: "Pony Diffusion V6 XL", downloads: 1_088_654,
    base_model: "Pony", nsfw: false, preview_image_url: null,
    file: { path: "ponyDiffusionV6XL_v6StartWithThisOne.safetensors", size: 6_938_041_050,
      sha256: "67ab2fd8ec439a89b3fedb15cc65f54336af163c7eb5e4f2acc98f090a29b0b3",
      download_url: "https://civitai.com/api/download/models/290640" } },
  { model_id: "443821", version_id: "2884631", name: "CyberRealistic Pony", downloads: 773_473,
    base_model: "Pony", nsfw: false, preview_image_url: null,
    file: { path: "cyberrealisticPony_v180Coreshift_pruned_fp16.safetensors", size: 6_938_041_288,
      sha256: "1d580c1c3f3612fa4db88af65372255582d5509ca0b28f85387273368301941b",
      download_url: "https://civitai.com/api/download/models/2884631" } },
  { model_id: "372465", version_id: "914390", name: "Pony Realism 🔮", downloads: 568_037,
    base_model: "Pony", nsfw: false, preview_image_url: null,
    file: { path: "ponyRealism_V22.safetensors", size: 7_105_348_856,
      sha256: "7c97ecf786a50a54835a22277c35703787b840e98c04c318a4e3fef9d3b463f7",
      download_url: "https://civitai.com/api/download/models/914390" } },
];

/** Families the mock "detects on read" for rows without a recorded
 *  `base_family`: model id → [family, source]. */
const DETECTED: Record<string, [string, string]> = {
  "m-sd15": ["sd15", "catalog"],
  "m-lora-wan-motion": ["wan22-5b", "header"],
  "m-lora-pony-eyes": ["pony", "name"],
  // Legacy `sdxl`; the name says Illustrious (a subfamily of the SDXL group).
  "m-hassaku": ["illustrious", "name"],
  "m-animagine": ["sdxl", "name"],
};

/** A row's family and how it was decided: recorded first, then detected. */
function familyOf(m: AnyRecord): [string, string] | null {
  if (m.base_family) return [String(m.base_family), String(m.family_source ?? "name")];
  return DETECTED[String(m.id)] ?? null;
}

const familyIdOf = (m: AnyRecord) => familyOf(m)?.[0] ?? null;

/** A registry family the mock has no stack for (chosen with "What base is
 *  this for?"): no base, no companions. */
function mockFamily(id: string): MockFamily | undefined {
  const own = FAMILIES.find((f) => f.id === id);
  if (own) return own;
  const known = BASE_FAMILIES.find((f) => f.id === id);
  if (!known) return undefined;
  return {
    id: known.id, label: known.label, arch_group: known.archGroup,
    runnable: known.notRunnable ? { kind: "no", reason: known.notRunnable } : { kind: "yes" },
    companions: [], civitai_label: known.label,
  };
}

function familyRef(f: MockFamily, source: string | null): AnyRecord {
  return { id: f.id, label: f.label, arch_group: f.arch_group, source, runnable: f.runnable };
}

function installedNeed(role: string, label: string, m: AnyRecord, madeFor: boolean, optional = false): AnyRecord {
  return { role, label, optional, status: { kind: "installed", model_id: m.id, name: m.name, made_for: madeFor } };
}

function isBase(m: AnyRecord): boolean {
  const roles = (m.roles as string[]) ?? [];
  return roles.includes("base_diffusion") || roles.includes("base_video");
}

function isLora(m: AnyRecord): boolean {
  return ((m.roles as string[]) ?? []).includes("lora");
}

function suggestion(f: MockFamily, optional: boolean, candidates: boolean): AnyRecord {
  if (f.base) {
    const [id, name, size] = f.base;
    return { role: "base", label: f.label, optional,
      status: { kind: "catalog", known_model_id: id, name, size_bytes: size, sha256: "", url: "" } };
  }
  return { role: "base", label: `${f.label} checkpoint`, optional,
    status: { kind: "findable", base_label: f.civitai_label,
      candidates: candidates && f.id === "pony" ? PONY_CANDIDATES : [], note: null } };
}

function baseNeeds(f: MockFamily, models: AnyRecord[], candidates: boolean): AnyRecord[] {
  const exact = checkpointsOf(f, models)[0];
  if (exact) return [installedNeed("base", f.label, exact, true)];
  // The architecture's catalogue base first, then the most used checkpoint.
  const works = models
    .filter((m) => isBase(m) && mockFamily(familyIdOf(m) ?? "")?.arch_group === f.arch_group)
    .sort((a, b) => rankBase(b) - rankBase(a))[0];
  if (works) return [installedNeed("base", f.label, works, false), suggestion(f, true, candidates)];
  return [suggestion(f, false, candidates)];
}

function companionNeeds(f: MockFamily, models: AnyRecord[]): AnyRecord[] {
  return f.companions.map(([role, id, name, size, have]) => {
    const m = have ? models.find((x) => x.id === have) : undefined;
    return m
      ? installedNeed(role, name, m, true)
      : { role, label: name, optional: false,
          status: { kind: "catalog", known_model_id: id, name, size_bytes: size, sha256: "", url: "" } };
  });
}

function missing(needs: AnyRecord[]): number {
  return needs
    .filter((n) => !n.optional)
    .reduce((sum, n) => {
      const s = n.status as AnyRecord;
      return s.kind === "catalog" ? sum + Number(s.size_bytes) : sum;
    }, 0);
}

/** Catalogue checkpoints rank first, then by use. */
const rankBase = (m: AnyRecord) =>
  (familyOf(m)?.[1] === "catalog" ? 1e9 : 0) + Number(m.use_count ?? 0);

/** Every checkpoint of a family, the catalogue one first, then most used. */
function checkpointsOf(f: MockFamily, models: AnyRecord[]): AnyRecord[] {
  return models
    .filter((m) => isBase(m) && familyIdOf(m) === f.id)
    .sort((a, b) => rankBase(b) - rankBase(a));
}

/** The trainer's Diffusers base is no checkpoint — say so on its group. */
function trainingNote(f: MockFamily, models: AnyRecord[]): string | null {
  const role = `training_base_${f.id.replace(/-/g, "_")}`;
  return models.some((m) => ((m.roles as string[]) ?? []).includes(role))
    ? `Your training base is installed; image generation with ${f.label} needs a single-file checkpoint.`
    : null;
}

export function mockLibraryPackages(models: AnyRecord[]): AnyRecord {
  const ids = [...new Set(models.map(familyIdOf).filter((x): x is string => !!x))];
  const order = BASE_FAMILIES.map((f) => f.id);
  ids.sort((a, b) => order.indexOf(a) - order.indexOf(b));
  const member = (m: AnyRecord) => ({ model: { ...m }, family_source: familyOf(m)?.[1] ?? "name" });
  const groups = ids.flatMap((id) => {
    const f = mockFamily(id);
    if (!f) return [];
    const checkpoints = checkpointsOf(f, models);
    const base = checkpoints[0];
    const loras = models.filter((m) => isLora(m) && familyIdOf(m) === f.id);
    if (!base && loras.length === 0) return [];
    const runnable = f.runnable.kind === "yes";
    const bNeeds = !runnable
      ? [{ role: "base", label: f.label, optional: false, status: { kind: "not_runnable", reason: f.runnable.reason } }]
      : base ? [] : baseNeeds(f, models, false);
    const companions = runnable ? companionNeeds(f, models) : [];
    return [{
      family: familyRef(f, null),
      base: base ? member(base) : null,
      checkpoints: checkpoints.map(member),
      base_needs: bNeeds,
      companions,
      loras: loras.map(member),
      missing_bytes: missing([...bNeeds, ...companions]),
      complete: runnable && !!base && companions.every((n) => (n.status as AnyRecord).kind === "installed"),
      base_choice_needed: !base && bNeeds.some((n) => !n.optional && (n.status as AnyRecord).kind === "findable"),
      note: base ? null : trainingNote(f, models),
    }];
  });
  const unknown = models
    .filter((m) => !familyOf(m) && (isLora(m) || isBase(m)))
    .map((m) => ({ model: { ...m }, kind: isLora(m) ? "lora" : "checkpoint" }));
  return { groups, unknown };
}

function kindOf(hint: unknown): string {
  const h = String(hint ?? "").toLowerCase();
  if (["lora", "locon", "dora"].includes(h)) return "lora";
  return h === "checkpoint" ? "checkpoint" : "other";
}

function packageFor(item: AnyRecord, familyId: string | null, source: string, models: AnyRecord[]): AnyRecord {
  if (item.kind === "companion") {
    // No base of its own; the stacks whose companion list names it use it.
    const usedBy = FAMILIES.filter((f) => f.companions.some((c) => c[4] === item.model_id));
    return { item, family: null, needs: [], missing_bytes: 0, verdict: { kind: "ready" },
      used_by: usedBy.map((f) => familyRef(f, null)) };
  }
  const f = familyId ? mockFamily(familyId) : undefined;
  if (!f) return { item, family: null, needs: [], missing_bytes: 0, verdict: { kind: "unknown_base" }, used_by: [] };
  if (f.runnable.kind === "no") {
    const reason = String(f.runnable.reason);
    return { item, family: familyRef(f, source),
      needs: [{ role: "base", label: f.label, optional: false, status: { kind: "not_runnable", reason } }],
      missing_bytes: 0, verdict: { kind: "not_runnable", reason }, used_by: [] };
  }
  const needs = [
    ...(item.kind === "checkpoint" ? [] : baseNeeds(f, models, true)),
    ...companionNeeds(f, models),
  ];
  const incomplete = needs.some(
    (n) => !n.optional && ["catalog", "findable"].includes(String((n.status as AnyRecord).kind)),
  );
  return { item, family: familyRef(f, source), needs, missing_bytes: missing(needs),
    verdict: { kind: incomplete ? "needs_download" : "ready" }, used_by: [] };
}

const COMPANION_ROLES = ["vae", "text_encoder", "clip_vision"];

function libraryKind(m: AnyRecord): string {
  if (isLora(m)) return "lora";
  if (isBase(m)) return "checkpoint";
  const roles = (m.roles as string[]) ?? [];
  return roles.some((r) => COMPANION_ROLES.includes(r)) ? "companion" : "other";
}

/** Civitai `baseModel` label → mock family id. */
const LABELS: Record<string, string> = {
  "SDXL 1.0": "sdxl", Pony: "pony", "Flux.1 D": "flux1", "Flux.2 Klein 9B": "flux2-klein-9b",
  "Wan Video 2.2 TI2V-5B": "wan22-5b", "Wan Video 2.2 I2V-A14B": "wan-14b",
};

export function mockResolvePackage(
  query: AnyRecord,
  models: AnyRecord[],
  civitai: AnyRecord[],
): AnyRecord {
  if (query.source === "library") {
    const m = models.find((x) => x.id === query.model_id);
    if (!m) throw new Error(`configuration error: packages: model ${String(query.model_id)} is not in the library`);
    const kind = libraryKind(m);
    const item = { name: m.name, kind, base_label: null, model_id: m.id, size_bytes: m.size_bytes };
    const fam = familyOf(m);
    return packageFor(item, fam?.[0] ?? null, fam?.[1] ?? "name", models);
  }
  const c = civitai.find((x) => x.id === query.model_id);
  if (!c) throw new Error("configuration error: registry: civitai: Civitai returned 404 Not Found");
  // The mock's cards carry the joined `baseModels`; the first label stands in
  // for the version's own `baseModel`.
  const label = String(c.base_model_family ?? "").split(",")[0].trim() || null;
  const item = { name: c.name, kind: kindOf(c.model_kind_hint), base_label: label, model_id: null, size_bytes: null };
  return packageFor(item, label ? (LABELS[label] ?? null) : null, "civitai", models);
}

export function mockSetBaseFamily(models: AnyRecord[], id: unknown, family: unknown): AnyRecord {
  const m = models.find((x) => x.id === id);
  if (!m) throw new Error(`configuration error: model ${String(id)} is not in the library`);
  if (!mockFamily(String(family))) {
    throw new Error(`configuration error: unknown base family ${JSON.stringify(family)}`);
  }
  m.base_family = family;
  m.family_source = "user";
  return { ...m };
}

export function mockSaveBaseFamilies(models: AnyRecord[], batch: AnyRecord[]): AnyRecord[] {
  return batch.map((row) => {
    const m = models.find((x) => x.id === row.model_id);
    if (!m) return { model_id: row.model_id, outcome: "skipped", reason: "not in the library" };
    if (m.family_source === "user") return { model_id: m.id, outcome: "kept_user_choice", reason: null };
    const detected = familyOf(m);
    if (!detected) return { model_id: m.id, outcome: "skipped", reason: "no family detected" };
    if (detected[0] !== row.family) {
      return { model_id: m.id, outcome: "skipped", reason: `detected ${detected[0]} now, not ${String(row.family)}` };
    }
    m.base_family = detected[0];
    m.family_source = detected[1];
    return { model_id: m.id, outcome: "written", reason: null };
  });
}

/** The origin columns a queued download carries (`download::DownloadOrigin`):
 *  the source id, and the base family its label maps to (unknown → null). */
export function mockDownloadOrigin(origin: AnyRecord | undefined): AnyRecord {
  if (!origin) return { origin: null, base_family: null, family_source: null };
  const civitai = origin.source === "civitai";
  const id = String(origin.model_id ?? "");
  const version = origin.version ? String(origin.version) : null;
  const label = origin.base_model ? String(origin.base_model) : "";
  const family = civitai ? (LABELS[label] ?? null) : null;
  return {
    origin: civitai
      ? `civitai:${id}${version ? `/${version}` : ""}`
      : `hf:${id}@${version ?? "main"}`,
    base_family: family,
    family_source: family ? "civitai" : null,
  };
}
