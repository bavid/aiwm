import type {
  FamilySource,
  GroupModel,
  LibraryGroup,
  LibraryPackages,
  Need,
  UnknownModel,
} from "../../lib/ipc";

/** Where a library group stands, as the card's one-line status says it. */
export type GroupState =
  | { kind: "not_runnable"; reason: string }
  | { kind: "ready" }
  /** No checkpoint of its own, but an installed base of the same
   *  architecture runs it (a Pony LoRA on SDXL). `suggestion` is the
   *  optional own base, if there is one to get. */
  | { kind: "works_with"; baseName: string; suggestion: Need | null }
  /** Required parts not installed; `bytes` counts the catalogue files only
   *  (a findable checkpoint's size is known once one is chosen).
   *  `chooseBase`: the base itself is missing and only findable — the
   *  person has to pick a checkpoint before any size is known. */
  | { kind: "missing"; needs: Need[]; bytes: number; chooseBase: boolean };

const isInstalled = (n: Need) => n.status.kind === "installed";

/** Required needs of the group that are not installed yet. */
export function missingNeeds(g: LibraryGroup): Need[] {
  return [...g.base_needs, ...g.companions].filter((n) => !n.optional && !isInstalled(n));
}

/** The installed base a baseless group runs on (made for another family of
 *  the same architecture), if any. */
export function worksWithBase(g: LibraryGroup): Need | null {
  return (
    g.base_needs.find((n) => n.status.kind === "installed" && !n.status.made_for) ?? null
  );
}

/** The optional "get its own base" suggestion (a findable or catalogue
 *  checkpoint), if any. */
export function baseSuggestion(g: LibraryGroup): Need | null {
  return (
    g.base_needs.find(
      (n) => n.optional && (n.status.kind === "findable" || n.status.kind === "catalog"),
    ) ?? null
  );
}

export function groupState(g: LibraryGroup): GroupState {
  if (g.family.runnable.kind === "no") return { kind: "not_runnable", reason: g.family.runnable.reason };
  const chooseBase = g.base_choice_needed;
  const missing = missingNeeds(g);
  if (missing.length > 0) return { kind: "missing", needs: missing, bytes: g.missing_bytes, chooseBase };
  if (g.complete) return { kind: "ready" };
  const works = worksWithBase(g);
  if (works && works.status.kind === "installed") {
    return { kind: "works_with", baseName: works.status.name, suggestion: baseSuggestion(g) };
  }
  // Nothing missing and nothing installed to run on — the backend reports
  // this only for a group whose base could not be placed; treat as missing.
  return { kind: "missing", needs: [], bytes: g.missing_bytes, chooseBase };
}

/** The "Unknown or unsupported" summary: "2 checkpoints (26 GB), 3 LoRAs". */
export function unknownSummary(unknown: readonly UnknownModel[]) {
  const checkpoints = unknown.filter((u) => u.kind === "checkpoint");
  return {
    checkpoints: checkpoints.length,
    checkpointBytes: checkpoints.reduce((sum, u) => sum + u.model.size_bytes, 0),
    loras: unknown.length - checkpoints.length,
  };
}

/** The library model a group's package is resolved from: its base when
 *  installed (the package is then its companions), else its first LoRA (the
 *  package is then the base plus companions). */
export function resolveTarget(g: LibraryGroup): string | null {
  return g.base?.model.id ?? g.loras[0]?.model.id ?? null;
}

/** The short name of what is missing: "Qwen3-8B text encoder, FLUX.2 VAE". */
export const missingText = (needs: readonly Need[]) =>
  needs
    .map((n) => (n.status.kind === "findable" ? `${n.label} (choose one)` : n.label))
    .join(", ");

/** Sources that are only a guess from the file name. */
export const isWeakSource = (s: FamilySource) => s === "name";

/** How a family was decided, as a phrase: "base from the file header". */
export const SOURCE_PHRASE: Record<FamilySource, string> = {
  civitai: "from Civitai",
  hf: "from Hugging Face",
  catalog: "from the catalogue",
  header: "from the file header",
  name: "guessed from the file name",
  user: "set by you",
};

/** One family the grouping inferred but the library has not recorded. */
export interface DetectedRow {
  modelId: string;
  name: string;
  family: string;
  familyLabel: string;
  source: FamilySource;
  weak: boolean;
}

/** Every group member whose family was inferred on read (not recorded in
 *  `base_family`, not the user's choice) — what "Save detected families"
 *  would persist. */
export function detectedRows(data: LibraryPackages): DetectedRow[] {
  return data.groups.flatMap((g) => {
    const members: GroupModel[] = [...g.checkpoints, ...g.loras];
    return members
      .filter((m) => m.family_source !== "user" && m.model.base_family !== g.family.id)
      .map((m) => ({
        modelId: m.model.id,
        name: m.model.name,
        family: g.family.id,
        familyLabel: g.family.label,
        source: m.family_source,
        weak: isWeakSource(m.family_source),
      }));
  });
}
