/** Dev-mock slice for storage locations (Plan 10): the `storage_locations`
 *  report and the core's up-front refusals of a chosen "store in" folder, so
 *  the Settings list and the prep / new-run forms behave in the browser
 *  preview as they do against the real core. Kept apart from `dev-mock.ts`
 *  (already far past a readable size). */

type AnyRecord = Record<string, unknown>;

/** How long the mock "walk" takes — long enough to see the loading state. */
const MEASURE_DELAY_MS = 700;

/** One drive for everything, like a default portable install. */
const VOLUME_FREE = 412_300_000_000;
const VOLUME_TOTAL = 1_800_000_000_000;

/** The rows the core reports as fixed (`configurable: false`). */
const FIXED_KEYS = new Set([
  "downloads",
  "logs",
  "exports",
  "voice_identities",
  "comfyui_data",
  "pending_import",
]);

function row(
  key: string,
  label: string,
  path: string,
  bytes: number,
  files: number,
  over: AnyRecord = {},
): AnyRecord {
  return {
    key,
    label,
    path,
    configurable: !FIXED_KEYS.has(key),
    exists: true,
    bytes,
    files,
    skipped: 0,
    volume_free_bytes: VOLUME_FREE,
    volume_total_bytes: VOLUME_TOTAL,
    ...over,
  };
}

/** `GET /storage/locations`, after a short delay (the real one walks every
 *  folder). The download staging folder does not exist yet, as on a fresh
 *  install with nothing downloading; one training file is unreadable. */
export async function mockStorageLocations(): Promise<AnyRecord[]> {
  await new Promise((resolve) => setTimeout(resolve, MEASURE_DELAY_MS));
  return [
    row("outputs", "Generated media (outputs)", "E:\\AI\\data\\outputs", 4_812_300_000, 214),
    row("datasets", "Dataset work folders", "E:\\AI\\data\\outputs\\datasets", 18_640_000_000, 9_212),
    row("training", "Training runs", "E:\\AI\\data\\training", 96_400_000_000, 1_340, { skipped: 1 }),
    row("models", "Model store", "E:\\AI\\models", 210_000_000_000, 18),
    row("runtimes", "Managed runtime installs", "E:\\AI\\data\\runtimes", 14_200_000_000, 3_801),
    row("cache", "Disposable cache", "E:\\AI\\data\\cache", 320_000_000, 640),
    row("downloads", "Download staging", "E:\\AI\\data\\.downloads", 0, 0, { exists: false }),
    row("logs", "Logs", "E:\\AI\\data\\logs", 184_000_000, 61),
    row("exports", "Backups", "E:\\AI\\data\\exports", 96_500_000, 3),
    row(
      "voice_identities",
      "Voice identities \u2014 user assets",
      "E:\\AI\\data\\voice-identities",
      12_400_000,
      4,
    ),
    row("comfyui_data", "ComfyUI scratch", "E:\\AI\\data\\comfyui-data", 1_210_000_000, 87),
    row("pending_import", "Pending import", "E:\\AI\\data\\.pending-import", 0, 0, {
      exists: false,
    }),
  ];
}

/** One cleanup entry (`CleanupEntry`), rows 0 unless given. */
function entry(
  id: string,
  label: string,
  files: number,
  bytes: number,
  detail: string[],
  rows = 0,
): AnyRecord {
  return { id, label, files, bytes, rows, detail };
}

function cleanupGroup(key: string, label: string, entries: AnyRecord[]): AnyRecord {
  return {
    key,
    label,
    entries,
    total_files: entries.reduce((n, e) => n + Number(e.files), 0),
    total_bytes: entries.reduce((n, e) => n + Number(e.bytes), 0),
  };
}

/** A mock group, as built once: its entries are removed by a real apply. */
interface MockGroup {
  key: string;
  label: string;
  entries: AnyRecord[];
}

/** The scan's group keys in display order — an apply works through the
 *  selections in this order, whatever order the request names them. */
const GROUP_KEYS = [
  "media_retention",
  "media_orphans",
  "discarded_frames",
  "unclaimed_dataset_folders",
  "missing_frame_rows",
  "finished_runs",
  "caches",
  "old_logs",
  "db_backups",
];

/** Where each group's files live in the demo data dir, for the preview's
 *  path list. */
const GROUP_FOLDER: Record<string, string> = {
  media_retention: "E:\\AI\\data\\outputs",
  media_orphans: "E:\\AI\\data\\outputs",
  discarded_frames: "E:\\AI\\data\\outputs\\datasets",
  unclaimed_dataset_folders: "E:\\AI\\data\\outputs\\datasets",
  missing_frame_rows: "",
  finished_runs: "E:\\AI\\data\\training",
  caches: "E:\\AI\\data\\cache",
  old_logs: "E:\\AI\\data\\logs",
  db_backups: "E:\\AI\\data\\exports",
};

/** The groups as first scanned; a real apply removes entries from them so
 *  the next scan shows what is left. Built on first use. */
let CLEANUP_GROUPS: MockGroup[] | null = null;
/** The cleanup history, newest first. */
const CLEANUP_LOG: AnyRecord[] = [];

function cleanupGroups(): MockGroup[] {
  if (!CLEANUP_GROUPS) CLEANUP_GROUPS = buildCleanupGroups();
  return CLEANUP_GROUPS;
}

/** `GET /cleanup/scan` (Plan 13), after the same delay: realistic groups
 *  for a machine that has rendered, curated and trained for a while, and
 *  the protected list a user would expect. Nothing here names a model. */
export async function mockCleanupScan(): Promise<AnyRecord> {
  await new Promise((resolve) => setTimeout(resolve, MEASURE_DELAY_MS));
  return {
    scanned_at: new Date().toISOString(),
    groups: cleanupGroups().map((g) => cleanupGroup(g.key, g.label, g.entries)),
    protected: [
      { what: "Model store E:\\AI\\models", reason: "models are never cleaned up here — delete a model from the library" },
      { what: "Runtime installs E:\\AI\\data\\runtimes", reason: "installed runtimes are managed from Settings, never cleaned up here" },
      { what: "Voice identities E:\\AI\\data\\voice-identities", reason: "your saved voices' reference clips are user assets" },
      { what: 'Dataset "Night market"', reason: 'dataset "Night market" is still being prepared (job 0199… is running) — wait for it to finish or cancel it first' },
      { what: 'Training run "street-v3"', reason: "the run is running — its folder stays until it finishes" },
      { what: 'Training run "street-v2": street-v2.safetensors', reason: "the only copy of the result — the LoRA is not in the library" },
      { what: "Download staging of flux2-klein-4b.safetensors", reason: "the download is paused" },
    ],
  };
}

/** The exact files an entry's apply would list: its detail lines when they
 *  are file names, else numbered files under the group's folder. */
function mockPaths(group: string, entry: AnyRecord): string[] {
  const files = Number(entry.files);
  if (files === 0) return [];
  const folder = GROUP_FOLDER[group] ?? "E:\\AI\\data";
  const detail = (entry.detail as string[]).filter(
    (line) => /\.[a-z0-9]{2,4}$/i.test(line) && !line.startsWith("E:\\") && !line.includes("…"),
  );
  if (detail.length === files) return detail.map((name) => `${folder}\\${entry.id}\\${name}`);
  if (files === 1) return [`${folder}\\${entry.id}`];
  return Array.from({ length: files }, (_, i) => `${folder}\\${entry.id}\\file-${String(i + 1).padStart(4, "0")}.png`);
}

/** `POST /cleanup/apply` (Plan 13): every selected entry that the last
 *  scan still offers is listed (dry run) or removed from the mock groups
 *  (real run, which also writes the history); an unknown id is skipped
 *  with `not_offered`, an unknown group is an error, like the core. */
export async function mockCleanupApply(body: AnyRecord): Promise<AnyRecord> {
  const dryRun = body.dry_run !== false;
  const selections = (Array.isArray(body.selections) ? body.selections : []) as AnyRecord[];
  for (const s of selections) {
    if (!GROUP_KEYS.includes(String(s.group))) {
      throw new Error(`configuration error: unknown cleanup group "${String(s.group)}"`);
    }
  }
  await new Promise((resolve) => setTimeout(resolve, MEASURE_DELAY_MS));
  const groups = cleanupGroups();
  const entries: AnyRecord[] = [];
  for (const key of GROUP_KEYS) {
    const ids = [
      ...new Set(
        selections
          .filter((s) => s.group === key)
          .flatMap((s) => (Array.isArray(s.entry_ids) ? s.entry_ids : []).map(String)),
      ),
    ];
    const group = groups.find((g) => g.key === key);
    for (const id of ids) {
      const found = group?.entries.find((e) => e.id === id);
      if (!group || !found) {
        entries.push({ group: key, id, label: id, files: 0, bytes: 0, rows: 0, skipped: [{ path: id, reason: "not_offered" }], paths: [] });
        continue;
      }
      entries.push({
        group: key,
        id,
        label: found.label,
        files: found.files,
        bytes: found.bytes,
        rows: found.rows,
        skipped: [],
        paths: mockPaths(key, found),
      });
      if (!dryRun) group.entries = group.entries.filter((e) => e.id !== id);
    }
  }
  if (!dryRun) {
    const ts = new Date().toISOString();
    for (const e of entries) {
      CLEANUP_LOG.unshift({
        id: `log-${CLEANUP_LOG.length + 1}-${Math.random().toString(36).slice(2, 8)}`,
        ts,
        group_key: e.group,
        entry_id: e.id,
        entry_label: e.label,
        deleted_files: e.files,
        freed_bytes: e.bytes,
        removed_rows: e.rows,
        skipped_count: (e.skipped as AnyRecord[]).length,
        detail: { skipped: e.skipped, paths: (e.paths as string[]).slice(0, 20) },
      });
    }
  }
  const sum = (field: string) => entries.reduce((n, e) => n + Number(e[field]), 0);
  return {
    dry_run: dryRun,
    deleted_files: sum("files"),
    freed_bytes: sum("bytes"),
    removed_rows: sum("rows"),
    skipped: entries.flatMap((e) => e.skipped as AnyRecord[]),
    entries,
  };
}

/** `GET /cleanup/log`: the newest rows, newest first (limit clamped like
 *  the core's `1..=200`). */
export async function mockCleanupLog(limit: number): Promise<AnyRecord[]> {
  const n = Math.min(200, Math.max(1, Math.round(limit)));
  return CLEANUP_LOG.slice(0, n).map((row) => ({ ...row }));
}

function buildCleanupGroups(): MockGroup[] {
  const group = (key: string, label: string, entries: AnyRecord[]): MockGroup => ({ key, label, entries });
  return [
    group("media_retention", "Generated media beyond the retention rule", [
      entry("0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b01.mp4", "0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b01.mp4", 1, 412_300_000, ["last modified 2026-07-02"]),
      entry("0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b02.png", "0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b02.png", 1, 4_100_000, ["last modified 2026-07-03"]),
    ]),
    group("media_orphans", "Generated media without a job", [
      entry("test-render.png", "test-render.png", 1, 3_900_000, ["last modified 2026-08-11"]),
      entry("hires-smoke.png", "hires-smoke.png", 1, 12_400_000, ["last modified 2026-09-01"]),
    ]),
    group("discarded_frames", "Discarded dataset frames", [
      entry("ds-1", "Studio portraits", 312, 1_640_000_000, ["work folder E:\\AI\\data\\outputs\\datasets\\0198…"], 312),
      entry("ds-2", "Street clips", 48, 210_000_000, ["work folder E:\\AI\\data\\outputs\\datasets\\0199…"], 48),
    ]),
    group("unclaimed_dataset_folders", "Dataset work folders without a dataset", [
      entry("0197aa00-deleted-prep", "0197aa00-deleted-prep", 1_960, 9_800_000_000, ["E:\\AI\\data\\outputs\\datasets\\0197aa00-deleted-prep"]),
    ]),
    group("missing_frame_rows", "Frame entries whose files are missing", [
      entry("ds-3", "missveronika milkpreg", 0, 0, ["1960 of 1960 frame entries"], 1_960),
    ]),
    group("finished_runs", "Finished training runs", [
      entry("run-a", "portrait-style-v1 — whole folder", 41, 6_200_000_000, ["E:\\AI\\data\\training\\run-a", 'the result LoRA "portrait-style-v1" is in the library']),
      entry("run-b", "street-v2 — checkpoints, optimizer state, samples and log", 26, 3_100_000_000, ["street-v2_000000250.safetensors", "street-v2_000000500.safetensors", "optimizer.pt", "train.log", "… and 22 more"]),
    ]),
    group("caches", "Caches and leftovers", [
      entry("cache", "Registry and runtime caches", 640, 320_000_000, ["E:\\AI\\data\\cache"]),
      entry("download-staging:0198e0", "Download staging 0198e0", 1, 1_900_000_000, ["E:\\AI\\data\\.downloads\\0198e0"]),
      entry("comfyui:input", "ComfyUI input leftovers", 12, 96_000_000, ["0198f1c2-…-9b01.png", "0198f1c2-…-9b02.png"]),
      entry("comfyui:temp", "ComfyUI temp leftovers", 30, 210_000_000, ["preview_00001.png", "preview_00002.png"]),
    ]),
    group("old_logs", "Logs older than 30 days", [
      entry("old-logs", "42 log file(s) older than 30 days", 42, 168_000_000, ["aiwm.log.2026-06-01", "aiwm.log.2026-06-02", "… and 40 more"]),
    ]),
    group("db_backups", "Database backups", [
      entry("aiwm-backup-2026-08-30.zip", "aiwm-backup-2026-08-30.zip (2026-08-30)", 1, 31_000_000, ["E:\\AI\\data\\exports\\aiwm-backup-2026-08-30.zip"]),
      entry("aiwm-backup-2026-09-15.zip", "aiwm-backup-2026-09-15.zip (2026-09-15)", 1, 33_500_000, ["E:\\AI\\data\\exports\\aiwm-backup-2026-09-15.zip"]),
    ]),
  ];
}

/** Like the core on Windows: a drive letter or a UNC path. */
const isAbsolute = (p: string) => /^([A-Za-z]:[\\/]|\\\\)/.test(p);
const isDriveRoot = (p: string) => /^[A-Za-z]:[\\/]*$/.test(p);
/** Lexical, case-insensitive "`inner` is `outer` or inside it". */
function sameOrInside(inner: string, outer: string): boolean {
  const norm = (p: string) => p.replace(/[\\/]+$/, "").toLowerCase().replace(/\//g, "\\");
  const i = norm(inner);
  const o = norm(outer);
  return i === o || i.startsWith(`${o}\\`);
}

/** The core's refusals of a prep run's "Store frames in" folder
 *  (`capability::dataset::location::check_data_dir`), same wording. */
export function checkPrepDataDir(dataDir: string, sourceRoot: string): void {
  if (!isAbsolute(dataDir)) {
    throw new Error(
      `configuration error: the folder to store frames in must be an absolute path, got ${JSON.stringify(dataDir)}`,
    );
  }
  if (isDriveRoot(dataDir)) {
    throw new Error(
      `configuration error: choose a folder to store frames in, not the whole drive ${dataDir}`,
    );
  }
  if (sameOrInside(dataDir, sourceRoot)) {
    throw new Error(
      `configuration error: frames cannot be stored inside the source folder ${sourceRoot} \u2014 choose a folder outside it`,
    );
  }
  if (sameOrInside(sourceRoot, dataDir)) {
    throw new Error(
      `configuration error: the source folder ${sourceRoot} lies inside ${dataDir} \u2014 choose a folder to store frames in that does not hold your source media`,
    );
  }
}

/** The core's refusal of a relative "Store run in" folder. */
export function checkRunDataDir(dataDir: string): void {
  if (!isAbsolute(dataDir)) {
    throw new Error(
      `configuration error: the folder to store the training run in must be an absolute path, got ${JSON.stringify(dataDir)}`,
    );
  }
}

/** The folder picker in the preview: there is no OS dialog, so a directory
 *  pick returns a fixed demo folder on a second drive; a file pick is
 *  cancelled, as before. */
export function mockDialogOpen(args: AnyRecord): string | null {
  const options = (args.options ?? {}) as AnyRecord;
  return options.directory === true ? "D:\\AIWM\\Picked" : null;
}
