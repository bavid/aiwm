import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { openFolder, useFolderPicker } from "../../lib/browse";
import type { AboutInfo, PathsUpdate, StorageLocation } from "../../lib/ipc";
import { useStorageLocations } from "../../lib/storage-locations";
import { formatBytes } from "../../lib/units";

/** The config field a location is moved with: one of the `[paths]` keys, or
 *  the model store's own `store_path`. */
export type PathField = keyof PathsUpdate | "store_path";

type Meta = {
  label: string;
  /** One line: what lands in this folder. */
  what: string;
  field: PathField | null;
  /** The effective path from `about()`, shown until the measurement lands. */
  aboutPath: ((a: AboutInfo) => string) | null;
};

/** Every location the app writes to, in display order. The core's report
 *  (`storageLocations`) supplies sizes; this supplies the words and the
 *  config field that moves each one. */
const LOCATIONS: Record<string, Meta> = {
  outputs: {
    label: "Generated media",
    what: "Images and video you render. Grows with every job.",
    field: "outputs_path",
    aboutPath: (a) => a.outputs_dir,
  },
  datasets: {
    label: "Dataset work folders",
    what: "Frames and previews from dataset prep, one folder per prep run.",
    field: "datasets_path",
    aboutPath: (a) => a.datasets_dir,
  },
  training: {
    label: "Training runs",
    what: "LoRA checkpoints, samples and logs, one folder per run.",
    field: "training_path",
    aboutPath: (a) => a.training_dir,
  },
  models: {
    label: "Model store",
    what: "Every imported model file — usually the largest folder.",
    field: "store_path",
    aboutPath: (a) => a.store_path,
  },
  runtimes: {
    label: "Runtime installs",
    what: "Managed llama.cpp, ComfyUI and trainer installs. Several GB.",
    field: "runtimes_path",
    aboutPath: (a) => a.runtimes_dir,
  },
  cache: {
    label: "Cache",
    what: "Registry cache. Disposable — safe to delete.",
    field: "cache_path",
    aboutPath: (a) => a.cache_dir,
  },
  downloads: {
    label: "Download staging",
    what: "Partial downloads while they transfer. Fixed, next to the data folder.",
    field: null,
    aboutPath: null,
  },
};

const ORDER = Object.keys(LOCATIONS);

/** Share of a drive in use at which the bar turns amber / red. */
const DRIVE_WARN_PCT = 70;
const DRIVE_CRIT_PCT = 90;
/** Words for a filling drive, so the state is never carried by colour alone. */
const DRIVE_QUALIFIER: Record<"ok" | "warn" | "crit", string> = {
  ok: "",
  warn: " — running low",
  crit: " — almost full",
};

type Props = {
  paths: PathsUpdate;
  storePath: string;
  /** The saved values — a differing field is marked "moves after restart". */
  savedPaths: PathsUpdate;
  savedStorePath: string;
  about: AboutInfo | null;
  onChange: (field: PathField, value: string) => void;
};

const fieldValue = (field: PathField, paths: PathsUpdate, storePath: string): string =>
  field === "store_path" ? storePath : paths[field];

/** Settings → Storage & data → "Data locations": every folder the app writes
 *  to with its size, file count and the free space of its drive; the
 *  configurable ones can be pointed elsewhere (applied after a restart). */
export function DataLocations({
  paths,
  storePath,
  savedPaths,
  savedStorePath,
  about,
  onChange,
}: Props) {
  const report = useStorageLocations();
  const [revealError, setRevealError] = useState<string | null>(null);
  const byKey = new Map((report.data ?? []).map((l) => [l.key, l]));
  const keys = [...ORDER, ...[...byKey.keys()].filter((k) => !ORDER.includes(k))];

  const reveal = async (path: string) => {
    setRevealError(await openFolder(path));
  };

  return (
    <section className="card set-group" aria-labelledby="data-locations-title">
      <header className="card__head">
        <h2 id="data-locations-title">Data locations</h2>
        <span className="card__sub">
          restart to apply <HelpHint area="settings" setting="data-locations" />
        </span>
      </header>
      <div className="loc-intro">
        <p className="muted">
          Changes apply after a restart. Existing data stays where it is; new datasets and
          training runs go to the new folder. Leave a field blank for the portable default next
          to the app.
        </p>
        <button
          type="button"
          className="loc-btn loc-btn--refresh"
          disabled={report.isLoading}
          onClick={report.refresh}
        >
          {report.isLoading ? "Measuring…" : "Refresh sizes"}
        </button>
      </div>
      {report.error && (
        <p className="settings__err" role="alert">
          Could not measure the folders: {report.error}
        </p>
      )}

      <ul className="loc-list" aria-busy={report.isLoading}>
        {keys.map((key) => {
          const meta = LOCATIONS[key];
          const measured = byKey.get(key) ?? null;
          const field = meta?.field ?? null;
          return (
            <LocationRow
              key={key}
              label={meta?.label ?? measured?.label ?? key}
              what={meta?.what ?? ""}
              path={measured?.path ?? (about && meta?.aboutPath ? meta.aboutPath(about) : "…")}
              measured={measured}
              isMeasuring={report.isLoading}
              field={field}
              draft={field ? fieldValue(field, paths, storePath) : ""}
              saved={field ? fieldValue(field, savedPaths, savedStorePath) : ""}
              onChange={(value) => field && onChange(field, value)}
              onReveal={(p) => void reveal(p)}
            />
          );
        })}
      </ul>
      {revealError && (
        <p className="settings__err" role="alert">
          {revealError}
        </p>
      )}
    </section>
  );
}

type RowProps = {
  label: string;
  what: string;
  path: string;
  measured: StorageLocation | null;
  isMeasuring: boolean;
  field: PathField | null;
  draft: string;
  saved: string;
  onChange: (value: string) => void;
  onReveal: (path: string) => void;
};

function LocationRow({
  label,
  what,
  path,
  measured,
  isMeasuring,
  field,
  draft,
  saved,
  onChange,
  onReveal,
}: RowProps) {
  const inputId = useId();
  const picker = useFolderPicker(onChange);
  const isPending = field !== null && draft.trim() !== saved.trim();
  // The model store has no blank "portable default" — it is always a path.
  const canReset = field !== null && field !== "store_path" && draft !== "";
  const exists = measured?.exists ?? false;
  // A saved folder the running app does not use yet (a blank one means the
  // default, whose path the UI cannot know until the restart).
  const isAwaitingRestart =
    !isPending &&
    saved.trim() !== "" &&
    measured !== null &&
    !samePath(saved, measured.path);

  return (
    <li className="loc" data-pending={isPending}>
      <div className="loc__head">
        <h3 className="loc__label">{label}</h3>
        <span className="loc__size numeric">{sizeText(measured, isMeasuring)}</span>
      </div>
      <div className="loc__sub">
        <p className="loc__what">{what}</p>
        <span className="loc__files numeric">{filesText(measured)}</span>
      </div>

      <div className="loc__where">
        <code className="loc__path">{path}</code>
        <button
          type="button"
          className="loc-btn"
          disabled={!exists}
          aria-label={`Open folder: ${label}`}
          aria-describedby={exists ? undefined : `${inputId}-missing`}
          onClick={() => onReveal(path)}
        >
          Open folder
        </button>
        {!exists && (
          <span id={`${inputId}-missing`} className="visually-hidden">
            The folder has not been created yet.
          </span>
        )}
      </div>

      <DriveBar measured={measured} />

      {field && (
        <div className="loc__edit">
          <label className="loc__edit-label" htmlFor={inputId}>
            {field === "store_path" ? "Folder" : "Custom folder"}
          </label>
          <input
            id={inputId}
            aria-label={`${field === "store_path" ? "Folder" : "Custom folder"} for ${label}`}
            type="text"
            value={draft}
            spellCheck={false}
            placeholder="default location"
            onChange={(e) => onChange(e.target.value)}
          />
          <button
            type="button"
            className="loc-btn"
            aria-label={`Choose… folder for ${label}`}
            onClick={picker.pick}
          >
            Choose…
          </button>
          {canReset && (
            <button
              type="button"
              className="loc-btn"
              aria-label={`Reset to default: ${label}`}
              onClick={() => onChange("")}
            >
              Reset to default
            </button>
          )}
        </div>
      )}
      {picker.error && (
        <p className="settings__err" role="alert">
          {picker.error}
        </p>
      )}
      {isPending && (
        <p className="loc__pending" role="status">
          After Save and a restart: {draft.trim() || "the default location"}
        </p>
      )}
      {isAwaitingRestart && (
        <p className="loc__pending" role="status">
          Saved — {saved.trim()} is used after a restart.
        </p>
      )}
    </li>
  );
}

/** Windows paths compared as Windows does: case-insensitive, either slash,
 *  trailing separators ignored. */
function samePath(a: string, b: string): boolean {
  const norm = (p: string) => p.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
  return norm(a) === norm(b);
}

function sizeText(m: StorageLocation | null, isMeasuring: boolean): string {
  if (!m) return isMeasuring ? "measuring…" : "—";
  if (!m.exists) return "not created yet";
  return formatBytes(m.bytes);
}

function filesText(m: StorageLocation | null): string {
  if (!m || !m.exists) return "";
  const files = `${m.files.toLocaleString()} ${m.files === 1 ? "file" : "files"}`;
  return m.skipped > 0 ? `${files} · ${m.skipped.toLocaleString()} unreadable` : files;
}

function DriveBar({ measured }: { measured: StorageLocation | null }) {
  const free = measured?.volume_free_bytes ?? null;
  const total = measured?.volume_total_bytes ?? null;
  if (free === null) return null;
  if (total === null || total <= 0) {
    return <p className="loc__drive-text numeric">{formatBytes(free)} free on this drive</p>;
  }
  const usedPct = Math.min(100, Math.max(0, ((total - free) / total) * 100));
  const load = usedPct >= DRIVE_CRIT_PCT ? "crit" : usedPct >= DRIVE_WARN_PCT ? "warn" : "ok";
  const text = `${formatBytes(free)} free of ${formatBytes(total)}${DRIVE_QUALIFIER[load]}`;
  return (
    <div className="loc__drive">
      <div
        className="loc__drive-track"
        role="meter"
        aria-label="Drive space in use"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(usedPct)}
        aria-valuetext={text}
      >
        <div
          className="loc__drive-fill"
          data-load={load}
          style={{ transform: `scaleX(${usedPct / 100})` }}
        />
      </div>
      <span className="loc__drive-text numeric" data-load={load}>
        {text}
      </span>
    </div>
  );
}
