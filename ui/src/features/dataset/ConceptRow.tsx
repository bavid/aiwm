import type { DatasetConcept } from "../../lib/ipc";

/** Below this a concept is unlikely to hold together during training — the
 *  same rule of thumb the plan documents for a per-concept frame budget. */
export const MIN_FRAMES_PER_CONCEPT = 20;

/** The one piece of curation advice both concept surfaces close with. */
export const CONCEPT_REMINDER =
  "Wide shots teach position, close-ups teach form — mix both, and vary the backgrounds.";

type Props = {
  concept: DatasetConcept;
  /** Omitted where the row is read-only (the Learn view's summary). */
  onDelete?: (concept: DatasetConcept) => void;
};

/** One concept as a list row: name, token, frame count and the two warnings
 *  (an ordinary-word token, and too few examples). Shared by the Concepts
 *  panel and the Learn view's summary so the wording stays in one place. */
export function ConceptRow({ concept, onDelete }: Props) {
  return (
    <li className="concepts__row">
      <div className="concepts__row-main">
        <span className="concepts__name">{concept.name}</span>
        <code className="concepts__token">{concept.token}</code>
        <span className="concepts__count">{concept.frame_count} frame(s)</span>
      </div>
      {concept.description && <p className="concepts__desc">{concept.description}</p>}
      {concept.token_warning && <p className="concepts__warn">{concept.token_warning}</p>}
      {concept.frame_count < MIN_FRAMES_PER_CONCEPT && (
        <p className="concepts__warn">
          fewer than {MIN_FRAMES_PER_CONCEPT} examples — the concept may not hold.
        </p>
      )}
      {onDelete && (
        <button type="button" className="chip concepts__delete" onClick={() => onDelete(concept)}>
          Delete
        </button>
      )}
    </li>
  );
}
