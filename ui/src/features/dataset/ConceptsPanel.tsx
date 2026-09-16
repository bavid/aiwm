import { useId, useState } from "react";
import { createConcept, deleteConcept, type DatasetConcept } from "../../lib/ipc";
import { tokenWarning } from "./tokens";

/** Below this a concept is unlikely to hold together during training — the
 *  same rule of thumb the plan documents for a per-concept frame budget. */
const MIN_FRAMES_PER_CONCEPT = 20;

type Props = {
  datasetId: string;
  concepts: DatasetConcept[];
  /** Refetch the concept list (and the grid's frame->concept map) after a
   *  create or delete — `createConcept` returns the bare row, without the
   *  computed `frame_count`/`token_warning`. */
  onChanged: () => void;
};

export function ConceptsPanel({ datasetId, concepts, onChanged }: Props) {
  const [name, setName] = useState("");
  const [token, setToken] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const nameId = useId();
  const tokenId = useId();
  const descId = useId();

  const draftWarning = token.trim() === "" ? null : tokenWarning(token);
  const canCreate = name.trim() !== "" && token.trim() !== "" && !busy;

  const create = async () => {
    if (!canCreate) return;
    setBusy(true);
    setError(null);
    try {
      await createConcept(datasetId, {
        name: name.trim(),
        token: token.trim(),
        description: description.trim() || undefined,
      });
      setName("");
      setToken("");
      setDescription("");
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (concept: DatasetConcept) => {
    const ok = window.confirm(
      `Delete the concept "${concept.name}" (${concept.token})? Its frames stay, they just lose the token.`,
    );
    if (!ok) return;
    try {
      await deleteConcept(concept.id);
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card concepts">
      <h3>Concepts</h3>
      <p className="concepts__intro">
        One token per thing the dataset should teach — a character, a prop, a place. Assign frames
        to it in the grid below.
      </p>

      {concepts.length > 0 && (
        <ul className="concepts__list">
          {concepts.map((c) => (
            <li key={c.id} className="concepts__row">
              <div className="concepts__row-main">
                <span className="concepts__name">{c.name}</span>
                <code className="concepts__token">{c.token}</code>
                <span className="concepts__count">{c.frame_count} frame(s)</span>
              </div>
              {c.description && <p className="concepts__desc">{c.description}</p>}
              {c.token_warning && <p className="concepts__warn">{c.token_warning}</p>}
              {c.frame_count < MIN_FRAMES_PER_CONCEPT && (
                <p className="concepts__warn">
                  fewer than {MIN_FRAMES_PER_CONCEPT} examples — the concept may not hold.
                </p>
              )}
              <button type="button" className="chip concepts__delete" onClick={() => remove(c)}>
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="concepts__form">
        <label className="datasetform__field" htmlFor={nameId}>
          <span>Name</span>
          <input
            id={nameId}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Kenji"
          />
        </label>
        <label className="datasetform__field" htmlFor={tokenId}>
          <span>Token</span>
          <input
            id={tokenId}
            type="text"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="kenji_xy"
            className="concepts__token-input"
          />
        </label>
        {draftWarning && <p className="concepts__warn">{draftWarning}</p>}
        <label className="datasetform__field" htmlFor={descId}>
          <span>Description (optional)</span>
          <input
            id={descId}
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="bare feet, visible"
          />
        </label>
        <button type="button" className="datasetform__go" onClick={create} disabled={!canCreate}>
          {busy ? "Creating…" : "Create concept"}
        </button>
        {error && <p className="dataset__err">{error}</p>}
      </div>

      <p className="concepts__reminder">
        Wide shots teach position, close-ups teach form — mix both, and vary the backgrounds.
      </p>
    </div>
  );
}
