import type { CleanupReport, CleanupSelection } from "../../lib/ipc";

/** The Cleanup page's data helpers: what each group is, and the selection
 *  (group key → entry ids) as an immutable value the page's state holds.
 *  No React here, so the rules are plain functions. */

/** Group labels as the core names them — the fallback for a history row
 *  whose group is not in the current scan (or before the first scan). */
export const GROUP_LABEL: Record<string, string> = {
  media_retention: "Generated media beyond the retention rule",
  media_orphans: "Generated media without a job",
  discarded_frames: "Discarded dataset frames",
  unclaimed_dataset_folders: "Dataset work folders without a dataset",
  missing_frame_rows: "Frame entries whose files are missing",
  finished_runs: "Finished training runs",
  caches: "Caches and leftovers",
  old_logs: "Logs older than 30 days",
  db_backups: "Database backups",
};

/** One "what this is" line per group, from the design's group table. */
export const GROUP_WHAT: Record<string, string> = {
  media_retention:
    "Images and videos in the generated-media folder that are older than the retention age or beyond the retention size set under Storage & data. The job stays in its list; only the file goes.",
  media_orphans:
    "Files in the generated-media folder that no job points to — leftovers of tests, crashed renders or manual copies.",
  discarded_frames:
    "Frames moved to Discard (excluded or rejected) on a dataset's curation board — the same cleanup as the dataset's own \"Clean up discarded\". Kept frames and source media stay.",
  unclaimed_dataset_folders:
    "Folders under the datasets folder that no dataset claims — from a dataset deleted while a file was locked, or a prep run that died.",
  missing_frame_rows:
    "Frame entries in the database whose files are gone; they show as broken thumbnails. Removing them frees no disk space — it tidies the dataset.",
  finished_runs:
    "Work folders of training runs that have finished: intermediate checkpoints, optimizer state, samples and the log. The whole folder is offered only when the result LoRA is in the library.",
  caches:
    "Disposable caches, download staging with no active download, ComfyUI input/temp/output leftovers older than an hour, and pending-import leftovers. All are rebuilt as needed.",
  old_logs: "Daily log files older than 30 days. Today's file is never listed.",
  db_backups: "Backup archives in the exports folder, by date — keep at least the newest.",
};

/** Entries shown per card before "Show all". */
export const ENTRY_LIST_CAP = 50;
/** Paths shown per entry in the preview dialog before "… and N more". */
export const PREVIEW_PATH_CAP = 20;

/** Group key → selected entry ids. Only immutable updates below. */
export type Selection = Readonly<Record<string, readonly string[]>>;

export const EMPTY_SELECTION: Selection = {};

/** `selection` with `id` of `group` added or removed. A group with no ids
 *  left is dropped, so "nothing selected" is `{}`. */
export function toggleEntry(selection: Selection, group: string, id: string): Selection {
  const current = selection[group] ?? [];
  const next = current.includes(id) ? current.filter((x) => x !== id) : [...current, id];
  return withGroup(selection, group, next);
}

/** `selection` with every id of `group` selected (`on`) or none. */
export function setGroup(
  selection: Selection,
  group: string,
  ids: readonly string[],
  on: boolean,
): Selection {
  return withGroup(selection, group, on ? [...ids] : []);
}

function withGroup(selection: Selection, group: string, ids: readonly string[]): Selection {
  const rest = Object.fromEntries(Object.entries(selection).filter(([k]) => k !== group));
  return ids.length === 0 ? rest : { ...rest, [group]: ids };
}

/** `selection` without the ids a new scan no longer offers, so a stale
 *  choice cannot be sent after a re-scan. */
export function pruneSelection(selection: Selection, report: CleanupReport): Selection {
  return Object.entries(selection).reduce<Selection>((acc, [group, ids]) => {
    const offered = new Set(report.groups.find((g) => g.key === group)?.entries.map((e) => e.id));
    return withGroup(acc, group, ids.filter((id) => offered.has(id)));
  }, EMPTY_SELECTION);
}

export interface SelectionTotals {
  items: number;
  files: number;
  bytes: number;
  rows: number;
}

/** What the selection adds up to, from the scan's own numbers. */
export function selectionTotals(report: CleanupReport | null, selection: Selection): SelectionTotals {
  const zero = { items: 0, files: 0, bytes: 0, rows: 0 };
  if (!report) return zero;
  return report.groups.reduce((acc, g) => {
    const ids = new Set(selection[g.key] ?? []);
    return g.entries
      .filter((e) => ids.has(e.id))
      .reduce(
        (t, e) => ({
          items: t.items + 1,
          files: t.files + e.files,
          bytes: t.bytes + e.bytes,
          rows: t.rows + e.rows,
        }),
        acc,
      );
  }, zero);
}

/** The apply request's `selections` — every group with at least one id,
 *  ids listed explicitly (a whole group sends all of its ids). */
export function toSelections(selection: Selection): CleanupSelection[] {
  return Object.entries(selection)
    .filter(([, ids]) => ids.length > 0)
    .map(([group, ids]) => ({ group, entry_ids: [...ids] }));
}

/** `count` with its noun: `1 file`, `2,048 files`. */
export function countLabel(count: number, singular: string, plural = `${singular}s`): string {
  return `${count.toLocaleString()} ${count === 1 ? singular : plural}`;
}

/** "X to free" from bytes and rows: the bytes always, the rows when any. */
export function freedLabel(bytes: number, rows: number, formatBytes: (b: number) => string): string {
  const parts = [formatBytes(bytes)];
  if (rows > 0) parts.push(`${countLabel(rows, "database entry", "database entries")}`);
  return parts.join(" and ");
}

/** An RFC 3339 stamp as the local time, for "Scanned at …"; the raw text
 *  when it does not parse. */
export function formatStamp(iso: string, style: "time" | "datetime" = "time"): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  return style === "time" ? at.toLocaleTimeString() : at.toLocaleString();
}
