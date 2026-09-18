import type { DatasetFrame } from "../../lib/ipc";

/** The filter values, in display order. `""` is "kept" — the pipeline writes an
 *  empty `rejection_reason` for a frame it did not drop. The rest mirror
 *  `core/src/capability/dataset/filter.rs::RejectionReason`. */
export const REJECTION_REASONS: readonly { value: string; label: string }[] = [
  { value: "", label: "Kept" },
  { value: "black", label: "Black" },
  { value: "transition", label: "Transition" },
  { value: "blur", label: "Blur" },
  { value: "duplicate", label: "Duplicate" },
  { value: "duplicate_global", label: "Duplicate (dataset)" },
  { value: "cap", label: "Cap" },
  { value: "unusable", label: "Unusable" },
];

/** Human label for one stored `rejection_reason`, for the badge on a card. */
export function rejectionLabel(reason: string): string {
  return REJECTION_REASONS.find((r) => r.value === reason)?.label ?? reason;
}

export function countByReason(frames: DatasetFrame[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const frame of frames) {
    counts[frame.rejection_reason] = (counts[frame.rejection_reason] ?? 0) + 1;
  }
  return counts;
}
