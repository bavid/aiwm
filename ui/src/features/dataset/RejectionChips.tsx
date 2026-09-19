import { useMemo } from "react";
import type { DatasetFrame } from "../../lib/ipc";
import { ALL_DISCARDED, EXCLUDED_BY_HAND } from "./curation";
import { countByReason, REJECTION_REASONS } from "./rejection";

type Props = {
  /** The Discard column's frames (excluded or rejected). */
  frames: readonly DatasetFrame[];
  active: string;
  onSelect: (filter: string) => void;
};

/** One filter at a time over the Discard column: everything, the frames
 *  excluded by hand, or one pipeline verdict. Empty verdicts are hidden. */
export function RejectionChips({ frames, active, onSelect }: Props) {
  // One pass over the column, re-run only when the frames change.
  const counts = useMemo(() => countByReason(frames), [frames]);
  const options = [
    { value: ALL_DISCARDED, label: "All", count: frames.length },
    { value: EXCLUDED_BY_HAND, label: "Excluded", count: counts[""] ?? 0 },
    ...REJECTION_REASONS.filter((r) => r.value !== "").map((r) => ({
      value: r.value,
      label: r.label,
      count: counts[r.value] ?? 0,
    })),
  ].filter((o) => o.value === ALL_DISCARDED || o.count > 0);

  if (options.length <= 2) return null;

  return (
    <div className="curation__filters" role="group" aria-label="Filter the Discard column">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          className="chip curation__filter"
          aria-pressed={active === o.value}
          onClick={() => onSelect(o.value)}
        >
          {o.label} <span className="curation__filter-count">{o.count.toLocaleString()}</span>
        </button>
      ))}
    </div>
  );
}
