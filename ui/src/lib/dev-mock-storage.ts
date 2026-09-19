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
    configurable: key !== "downloads",
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
