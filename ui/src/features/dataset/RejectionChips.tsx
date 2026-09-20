import { useId, useMemo } from "react";
import { HelpHint } from "../../components/HelpHint";
import type { DatasetFrame } from "../../lib/ipc";
import { ALL_DISCARDED, EXCLUDED_BY_HAND } from "./curation";
import { countByReason, REJECTION_REASONS } from "./rejection";

/** The help entry for a filter value; "All" has none. */
const HINT_KEY: Record<string, string> = {
  [EXCLUDED_BY_HAND]: "excluded",
  black: "reason-black",
  transition: "reason-transition",
  blur: "reason-blur",
  duplicate: "reason-duplicate",
  duplicate_global: "reason-duplicate-global",
  cap: "reason-cap",
  unusable: "reason-unusable",
};

/** One chip with its `?`: the hint sits outside the button, so its name
 *  stays out of the chip's, and describes the chip while collapsed. */
function ReasonChip({
  value,
  label,
  count,
  isActive,
  onSelect,
}: {
  value: string;
  label: string;
  count: number;
  isActive: boolean;
  onSelect: (filter: string) => void;
}) {
  const id = useId();
  const hintKey = HINT_KEY[value];
  return (
    <span className="curation__filter-item">
      <button
        id={id}
        type="button"
        className="chip curation__filter"
        aria-pressed={isActive}
        onClick={() => onSelect(value)}
      >
        {label} <span className="curation__filter-count">{count.toLocaleString()}</span>
      </button>
      {hintKey === "excluded" && <HelpHint area="dataset" setting="excluded" describes={id} />}
      {hintKey === "reason-black" && <HelpHint area="dataset" setting="reason-black" describes={id} />}
      {hintKey === "reason-transition" && (
        <HelpHint area="dataset" setting="reason-transition" describes={id} />
      )}
      {hintKey === "reason-blur" && <HelpHint area="dataset" setting="reason-blur" describes={id} />}
      {hintKey === "reason-duplicate" && (
        <HelpHint area="dataset" setting="reason-duplicate" describes={id} />
      )}
      {hintKey === "reason-duplicate-global" && (
        <HelpHint area="dataset" setting="reason-duplicate-global" describes={id} />
      )}
      {hintKey === "reason-cap" && <HelpHint area="dataset" setting="reason-cap" describes={id} />}
      {hintKey === "reason-unusable" && (
        <HelpHint area="dataset" setting="reason-unusable" describes={id} />
      )}
    </span>
  );
}

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
        <ReasonChip
          key={o.value}
          value={o.value}
          label={o.label}
          count={o.count}
          isActive={active === o.value}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}
