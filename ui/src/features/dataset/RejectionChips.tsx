import { useMemo } from "react";
import type { DatasetFrame } from "../../lib/ipc";
import { countByReason, REJECTION_REASONS } from "./rejection";

type Props = {
  frames: DatasetFrame[];
  active: string;
  onSelect: (reason: string) => void;
};

/** One filter at a time over the curation set. Reasons with no frames are
 *  hidden so a clean run shows a single chip — except "Kept", which stays
 *  visible as the default even at zero. */
export function RejectionChips({ frames, active, onSelect }: Props) {
  // One pass over the whole curation set, re-run only when the frames change --
  // not on every keystroke elsewhere in the tab.
  const counts = useMemo(() => countByReason(frames), [frames]);
  const shown = REJECTION_REASONS.filter((r) => r.value === "" || (counts[r.value] ?? 0) > 0);

  return (
    <div className="dataset__filters" role="group" aria-label="Filter by pipeline verdict">
      {shown.map((r) => (
        <button
          key={r.value || "kept"}
          type="button"
          className="chip"
          aria-pressed={active === r.value}
          onClick={() => onSelect(r.value)}
        >
          {r.label} ({counts[r.value] ?? 0})
        </button>
      ))}
    </div>
  );
}
