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

/** `GET /cleanup/scan` (Plan 13), after the same delay: realistic groups
 *  for a machine that has rendered, curated and trained for a while, and
 *  the protected list a user would expect. Nothing here names a model. */
export async function mockCleanupScan(): Promise<AnyRecord> {
  await new Promise((resolve) => setTimeout(resolve, MEASURE_DELAY_MS));
  return {
    scanned_at: new Date().toISOString(),
    groups: [
      cleanupGroup("media_retention", "Generated media beyond the retention rule", [
        entry("0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b01.mp4", "0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b01.mp4", 1, 412_300_000, ["last modified 2026-07-02"]),
        entry("0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b02.png", "0198f1c2-4a7e-7c1a-9c2d-2f0e6d1a9b02.png", 1, 4_100_000, ["last modified 2026-07-03"]),
      ]),
      cleanupGroup("media_orphans", "Generated media without a job", [
        entry("test-render.png", "test-render.png", 1, 3_900_000, ["last modified 2026-08-11"]),
        entry("hires-smoke.png", "hires-smoke.png", 1, 12_400_000, ["last modified 2026-09-01"]),
      ]),
      cleanupGroup("discarded_frames", "Discarded dataset frames", [
        entry("ds-1", "Studio portraits", 312, 1_640_000_000, ["work folder E:\\AI\\data\\outputs\\datasets\\0198…"], 312),
        entry("ds-2", "Street clips", 48, 210_000_000, ["work folder E:\\AI\\data\\outputs\\datasets\\0199…"], 48),
      ]),
      cleanupGroup("unclaimed_dataset_folders", "Dataset work folders without a dataset", [
        entry("0197aa00-deleted-prep", "0197aa00-deleted-prep", 1_960, 9_800_000_000, ["E:\\AI\\data\\outputs\\datasets\\0197aa00-deleted-prep"]),
      ]),
      cleanupGroup("missing_frame_rows", "Frame entries whose files are missing", [
        entry("ds-3", "missveronika milkpreg", 0, 0, ["1960 of 1960 frame entries"], 1_960),
      ]),
      cleanupGroup("finished_runs", "Finished training runs", [
        entry("run-a", "portrait-style-v1 — whole folder", 41, 6_200_000_000, ["E:\\AI\\data\\training\\run-a", 'the result LoRA "portrait-style-v1" is in the library']),
        entry("run-b", "street-v2 — checkpoints, optimizer state, samples and log", 26, 3_100_000_000, ["street-v2_000000250.safetensors", "street-v2_000000500.safetensors", "optimizer.pt", "train.log", "… and 22 more"]),
      ]),
      cleanupGroup("caches", "Caches and leftovers", [
        entry("cache", "Registry and runtime caches", 640, 320_000_000, ["E:\\AI\\data\\cache"]),
        entry("download-staging:0198e0", "Download staging 0198e0", 1, 1_900_000_000, ["E:\\AI\\data\\.downloads\\0198e0"]),
        entry("comfyui:input", "ComfyUI input leftovers", 12, 96_000_000, ["0198f1c2-…-9b01.png", "0198f1c2-…-9b02.png"]),
        entry("comfyui:temp", "ComfyUI temp leftovers", 30, 210_000_000, ["preview_00001.png", "preview_00002.png"]),
      ]),
      cleanupGroup("old_logs", "Logs older than 30 days", [
        entry("old-logs", "42 log file(s) older than 30 days", 42, 168_000_000, ["aiwm.log.2026-06-01", "aiwm.log.2026-06-02", "… and 40 more"]),
      ]),
      cleanupGroup("db_backups", "Database backups", [
        entry("aiwm-backup-2026-08-30.zip", "aiwm-backup-2026-08-30.zip (2026-08-30)", 1, 31_000_000, ["E:\\AI\\data\\exports\\aiwm-backup-2026-08-30.zip"]),
        entry("aiwm-backup-2026-09-15.zip", "aiwm-backup-2026-09-15.zip (2026-09-15)", 1, 33_500_000, ["E:\\AI\\data\\exports\\aiwm-backup-2026-09-15.zip"]),
      ]),
    ],
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
