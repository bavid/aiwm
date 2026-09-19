import { useId, type ReactNode } from "react";
import { useFolderPicker } from "../lib/browse";
import {
  driveOf,
  driveSpace,
  useStorageLocations,
  type DriveSpace,
} from "../lib/storage-locations";
import "./storage-dir-field.css";

/** The core counts its free-space minimums (and words its refusal) in
 *  1024³-byte gigabytes. This line judges *and* prints in that same unit,
 *  labelled GiB, so the free figure, the minimum and the core's own refusal
 *  never disagree — a decimal "5.3 GB free" beside "5 GB needed" could. */
const BYTES_PER_GIB = 1024 ** 3;

/** Bytes as binary gigabytes (terabytes from 1024 GiB): `412.3 GiB`. */
function formatGiBytes(bytes: number): string {
  const gib = bytes / BYTES_PER_GIB;
  if (gib >= 1024) return `${(gib / 1024).toFixed(1)} TiB`;
  return `${gib.toFixed(1)} GiB`;
}

type Props = {
  /** "Store frames in", "Store run in". */
  label: string;
  /** The chosen folder; blank = the default location. */
  value: string;
  onChange: (next: string) => void;
  /** The effective default folder (from `about()`), shown as placeholder. */
  defaultDir: string | null;
  /** The storage location the default belongs to (`datasets` | `training`). */
  locationKey: string;
  /** The core refuses to start below this much free space on the drive, in
   *  GiB (the core's own unit). */
  minFreeGiB: number;
  /** Why and what happens — one or two short sentences. */
  help: ReactNode;
};

type FreeLine = { text: string; tone: "ok" | "warn" | "muted" };

function freeLine(
  value: string,
  defaultDir: string | null,
  locationKey: string,
  minFreeGiB: number,
  state: ReturnType<typeof useStorageLocations>,
): FreeLine {
  const need = `at least ${minFreeGiB} GiB needed`;
  const later: FreeLine = { text: `Free space is checked when you start — ${need}.`, tone: "muted" };
  if (state.isLoading && !state.data) return { text: "Checking free space…", tone: "muted" };
  if (!state.data) return later;

  const custom = value.trim();
  let space: DriveSpace | null = null;
  if (custom === "") {
    const loc = state.data.find((l) => l.key === locationKey);
    const drive = driveOf(loc?.path ?? defaultDir ?? "") ?? "its drive";
    if (loc && loc.volume_free_bytes !== null) {
      space = { drive, free: loc.volume_free_bytes, total: loc.volume_total_bytes };
    }
  } else {
    space = driveSpace(custom, state.data);
  }
  if (!space) return later;

  const total = space.total !== null ? ` of ${formatGiBytes(space.total)}` : "";
  const text = `${formatGiBytes(space.free)} free${total} on ${space.drive}`;
  if (space.free < minFreeGiB * BYTES_PER_GIB) {
    return { text: `${text} — too little, ${need}.`, tone: "warn" };
  }
  return { text: `${text}.`, tone: "ok" };
}

/** The optional "store … in" folder row of the dataset-prep and new-run
 *  forms: blank keeps the default location, a folder puts this one job's
 *  data there. Shows the free space of the drive it will land on. */
export function StorageDirField({
  label,
  value,
  onChange,
  defaultDir,
  locationKey,
  minFreeGiB,
  help,
}: Props) {
  const inputId = useId();
  const freeId = useId();
  const helpId = useId();
  const locations = useStorageLocations();
  const free = freeLine(value, defaultDir, locationKey, minFreeGiB, locations);
  const picker = useFolderPicker(onChange);
  const isCustom = value.trim() !== "";

  return (
    <div className="storedir">
      <div className="storedir__head">
        <label htmlFor={inputId} className="storedir__label">
          {label}
        </label>
        <span className="storedir__badge" data-custom={isCustom}>
          {isCustom ? "custom folder" : "default"}
        </span>
      </div>
      <div className="storedir__row">
        <input
          id={inputId}
          type="text"
          value={value}
          spellCheck={false}
          placeholder={defaultDir ?? "the default folder"}
          aria-describedby={`${freeId} ${helpId}`}
          onChange={(e) => onChange(e.target.value)}
        />
        <button
          type="button"
          className="storedir__btn"
          aria-label={`Choose a folder to ${label.toLowerCase()}`}
          onClick={picker.pick}
        >
          Choose…
        </button>
        {isCustom && (
          <button
            type="button"
            className="storedir__btn"
            aria-label="Reset to the default folder"
            onClick={() => onChange("")}
          >
            Reset
          </button>
        )}
      </div>
      {picker.error && (
        <p className="storedir__free" data-tone="warn" role="alert">
          {picker.error}
        </p>
      )}
      <p id={freeId} className="storedir__free numeric" data-tone={free.tone} aria-live="polite">
        {free.text}
      </p>
      <p id={helpId} className="storedir__help">
        {help}
      </p>
    </div>
  );
}
