import type { DatasetFrame, SkippedFile } from "../../lib/ipc";

/** The two columns of the curation board. */
export type ColumnId = "keep" | "discard";

export const COLUMN_LABEL: Record<ColumnId, string> = {
  keep: "Keep",
  discard: "Discard",
};

export const isColumnId = (value: string): value is ColumnId =>
  value === "keep" || value === "discard";

export const otherColumn = (column: ColumnId): ColumnId =>
  column === "keep" ? "discard" : "keep";

/** Discard = excluded by hand or rejected by a filter; the export skips both. */
export const isDiscarded = (frame: DatasetFrame): boolean =>
  frame.excluded || frame.rejection_reason !== "";

export const columnOf = (frame: DatasetFrame): ColumnId =>
  isDiscarded(frame) ? "discard" : "keep";

/** The Discard column's filter: `ALL_DISCARDED`, `EXCLUDED_BY_HAND` (excluded
 *  with no filter verdict) or a stored `rejection_reason`. */
export const ALL_DISCARDED = "*";
export const EXCLUDED_BY_HAND = "excluded";

export function matchesDiscardFilter(frame: DatasetFrame, filter: string): boolean {
  if (filter === ALL_DISCARDED) return true;
  if (filter === EXCLUDED_BY_HAND) return frame.rejection_reason === "";
  return frame.rejection_reason === filter;
}

/** "1 frame" / "12 frames" — the unit every count on the board uses. */
export const framesLabel = (count: number): string =>
  `${count.toLocaleString()} ${count === 1 ? "frame" : "frames"}`;

/** "1 file" / "12 files". */
export const filesLabel = (count: number): string =>
  `${count.toLocaleString()} ${count === 1 ? "file" : "files"}`;

/** The message of a rejected IPC call, verbatim. The core's refusals (an
 *  active training run, …) are written for the curator, so they are shown as
 *  they are — without the `Error: ` prefix `String(error)` would add. */
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

const SKIP_REASON_LABEL: Record<string, string> = {
  outside_app_folders: "outside the app's folders — not the app's to delete",
  source_file: "a source file — never deleted",
  in_use: "still used by another frame",
  used_by_other_dataset: "used by another dataset",
  not_a_file: "not a file",
};

/** Human text for one `SkippedFile.reason`; `"error: …"` is shown verbatim. */
export function skipReasonLabel(file: SkippedFile): string {
  return SKIP_REASON_LABEL[file.reason] ?? file.reason;
}

/** `items` in consecutive slices of at most `size` (the last may be shorter). */
export function chunk<T>(items: readonly T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
  return out;
}
