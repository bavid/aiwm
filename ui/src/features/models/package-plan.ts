import type {
  CheckpointCandidate,
  DownloadOrigin,
  EnqueueDownloadBody,
  KnownModel,
  Need,
  Package,
  RegistryDetails,
  RegistryFile,
} from "../../lib/ipc";
import { knownModelDownload } from "./stack-install";

/** Civitai `baseModel` labels this app cannot run, and why — a copy of the
 *  `Runnable::No` entries of `core/src/model/family.rs` (`FAMILIES`).
 *
 *  Why a client-side copy instead of resolving each card: `resolve_package`
 *  fetches the model's details from Civitai (and, for a findable base, a
 *  checkpoint search) — one or two network calls per card, twenty cards per
 *  search, against a rate-limited API. The card only needs "is this base
 *  known to be unrunnable", which the joined label already answers. The
 *  dialog still resolves the real package, so a stale copy here can only
 *  miss a warning on the card, never download the wrong thing. Keep in
 *  sync with the registry when a family's `runnable` changes. */
const NOT_RUNNABLE: readonly { family: string; reason: string; labels: readonly string[] }[] = [
  {
    family: "Wan 14B",
    reason: "does not fit 16 GB — this app runs the 5B",
    labels: [
      "Wan Video 2.2 I2V-A14B",
      "Wan Video 2.2 T2V-A14B",
      "Wan Video 14B t2v",
      "Wan Video 14B i2v 480p",
      "Wan Video 14B i2v 720p",
    ],
  },
  {
    family: "LTX-2",
    reason:
      "LTX-2 is a different, much larger audio-video model — this app runs LTX-Video 0.9.5 (2B)",
    labels: ["LTXV2", "LTXV 2.3"],
  },
];

export interface CardBaseWarning {
  /** The Civitai label as the card shows it. */
  label: string;
  family: string;
  reason: string;
  /** Every version targets an unrunnable base (not just some of them). */
  every: boolean;
}

const notRunnableFor = (label: string) => {
  const l = label.trim().toLowerCase();
  return NOT_RUNNABLE.find((f) => f.labels.some((x) => x.toLowerCase() === l)) ?? null;
};

/** The card-level "this will not load" line, from the model's joined
 *  `baseModels` facet (`"Pony, SD 1.5"`). `null` when nothing listed is
 *  known to be unrunnable. */
export function cardBaseWarning(baseModelFamily: string | null): CardBaseWarning | null {
  const labels = (baseModelFamily ?? "")
    .split(",")
    .map((l) => l.trim())
    .filter(Boolean);
  const hits = labels.flatMap((label) => {
    const hit = notRunnableFor(label);
    return hit ? [{ label, family: hit.family, reason: hit.reason }] : [];
  });
  if (hits.length === 0) return null;
  return { ...hits[0], every: hits.length === labels.length };
}

/** The file a Civitai pick downloads: the primary version's first
 *  `.safetensors` (what the core resolver matches against the library),
 *  else its first file. */
export function primaryFile(details: RegistryDetails): RegistryFile | null {
  return (
    details.files.find((f) => f.path.toLowerCase().endsWith(".safetensors")) ??
    details.files[0] ??
    null
  );
}

/** The Civitai `baseModel` of the version whose files `details` lists (the
 *  primary one), else the first label of the model's joined facet. */
export function primaryBaseLabel(details: RegistryDetails): string | null {
  const own = details.versions?.find((v) => v.id === details.revision)?.base_model;
  if (own) return own;
  const first = (details.base_model_family ?? "").split(",")[0].trim();
  return first || null;
}

/** What the person picked for one need: whether to fetch it, and — for a
 *  findable base — which of the offered checkpoints. */
export interface NeedChoice {
  include: boolean;
  candidate: number;
}

/** Required needs start ticked, suggestions (optional) start unticked; the
 *  first (most downloaded) checkpoint is preselected. */
export const initialChoices = (needs: readonly Need[]): NeedChoice[] =>
  needs.map((n) => ({ include: !n.optional, candidate: 0 }));

/** One file the "+ missing" button would queue. */
export interface PlannedFile {
  label: string;
  body: EnqueueDownloadBody;
}

/** A checkpoint candidate's queue request, recording where it came from so
 *  the import stores its family. `null` when Civitai gave no file/URL. */
function candidateDownload(c: CheckpointCandidate): EnqueueDownloadBody | null {
  if (!c.file?.download_url) return null;
  return {
    url: c.file.download_url,
    filename: c.file.path,
    model_type: "checkpoint",
    sha256: c.file.sha256 ?? undefined,
    size_bytes: c.file.size,
    origin: {
      source: "civitai",
      model_id: c.model_id,
      version: c.version_id,
      base_model: c.base_model,
    },
  };
}

/** The download a need would start with this choice, or `null` when it
 *  needs nothing (installed), cannot be fetched (not runnable, no file), or
 *  its catalog entry is not in the loaded catalogue. */
export function needDownload(
  need: Need,
  choice: NeedChoice,
  known: readonly KnownModel[],
): PlannedFile | null {
  const s = need.status;
  if (s.kind === "catalog") {
    const member = known.find((k) => k.id === s.known_model_id);
    return member ? { label: s.name, body: knownModelDownload(member) } : null;
  }
  if (s.kind === "findable") {
    const c = s.candidates[choice.candidate];
    const body = c ? candidateDownload(c) : null;
    return c && body ? { label: c.name, body } : null;
  }
  return null;
}

/** Every file the "+ missing" button queues, in need order. */
export function plannedNeeds(
  pkg: Package,
  choices: readonly NeedChoice[],
  known: readonly KnownModel[],
): PlannedFile[] {
  return pkg.needs.flatMap((n, i) => {
    const choice = choices[i];
    if (!choice?.include) return [];
    const planned = needDownload(n, choice, known);
    return planned ? [planned] : [];
  });
}

export const plannedBytes = (files: readonly PlannedFile[]) =>
  files.reduce((sum, f) => sum + (f.body.size_bytes ?? 0), 0);

/** The Civitai pick's own queue request, with its origin. */
export function itemDownload(
  details: RegistryDetails,
  file: RegistryFile,
  modelType: EnqueueDownloadBody["model_type"],
  baseLabel: string | null,
): EnqueueDownloadBody {
  const origin: DownloadOrigin = {
    source: "civitai",
    model_id: details.id,
    version: details.revision,
    ...(baseLabel ? { base_model: baseLabel } : {}),
  };
  return {
    url: file.download_url,
    filename: file.path,
    model_type: modelType,
    sha256: file.sha256 ?? undefined,
    size_bytes: file.size_bytes,
    origin,
  };
}
