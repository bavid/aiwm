import { useId } from "react";
import type { DatasetConcept } from "../../lib/ipc";

export type AssignState =
  | { kind: "idle" }
  | { kind: "done"; text: string }
  | { kind: "error"; text: string };

type Props = {
  concepts: DatasetConcept[];
  conceptId: string;
  onConceptIdChange: (id: string) => void;
  onAssign: () => void;
  onRemove: () => void;
  state: AssignState;
};

/** The concept row of the board's selection bar: pick a concept, attach or
 *  detach the whole selection. */
export function SelectionToolbar({
  concepts,
  conceptId,
  onConceptIdChange,
  onAssign,
  onRemove,
  state,
}: Props) {
  const assignSelectId = useId();

  return (
    <div className="curation__concepts">
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
      {state.kind === "done" && <span className="dataset__toolbar-status">{state.text}</span>}
      {state.kind === "error" && <span className="dataset__err">{state.text}</span>}
    </div>
  );
}
