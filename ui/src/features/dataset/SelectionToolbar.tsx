import { useId } from "react";
import type { DatasetConcept } from "../../lib/ipc";

export type AssignState =
  | { kind: "idle" }
  | { kind: "done"; text: string }
  | { kind: "error"; text: string };

type Props = {
  selectedCount: number;
  concepts: DatasetConcept[];
  conceptId: string;
  onConceptIdChange: (id: string) => void;
  onAssign: () => void;
  onRemove: () => void;
  onClear: () => void;
  state: AssignState;
};

/** The bar above the curation grid, shown once at least one item is selected:
 *  pick a concept, attach or detach the whole selection, clear it. */
export function SelectionToolbar({
  selectedCount,
  concepts,
  conceptId,
  onConceptIdChange,
  onAssign,
  onRemove,
  onClear,
  state,
}: Props) {
  const assignSelectId = useId();

  return (
    <div className="card dataset__toolbar">
      <span className="dataset__toolbar-count">{selectedCount} selected</span>
      <label className="datasetform__field datasetform__field--inline" htmlFor={assignSelectId}>
        <span className="dataset__toolbar-label">Concept</span>
        <select
          id={assignSelectId}
          value={conceptId}
          onChange={(e) => onConceptIdChange(e.target.value)}
        >
          <option value="">Choose a concept…</option>
          {concepts.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} ({c.token})
            </option>
          ))}
        </select>
      </label>
      <button type="button" className="chip" disabled={!conceptId} onClick={onAssign}>
        Assign to concept
      </button>
      <button type="button" className="chip" disabled={!conceptId} onClick={onRemove}>
        Remove from concept
      </button>
      <button type="button" className="chip" onClick={onClear}>
        Clear selection
      </button>
      {state.kind === "done" && <span className="dataset__toolbar-status">{state.text}</span>}
      {state.kind === "error" && <span className="dataset__err">{state.text}</span>}
    </div>
  );
}
