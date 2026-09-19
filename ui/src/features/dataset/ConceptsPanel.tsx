import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { createConcept, deleteConcept, type DatasetConcept } from "../../lib/ipc";
import { CONCEPT_REMINDER, ConceptRow } from "./ConceptRow";
import { tokenWarning } from "./tokens";

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
            <ConceptRow key={c.id} concept={c} onDelete={remove} />
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
        <div className="datasetform__field">
          <span>
            <label htmlFor={tokenId}>Token</label>
            <HelpHint area="dataset" setting="concept" describes={tokenId} />
          </span>
          <input
            id={tokenId}
            type="text"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="kenji_xy"
            className="concepts__token-input"
          />
        </div>
        {draftWarning && <p className="concepts__warn">{draftWarning}</p>}
        <div className="datasetform__field">
          <span>
            <label htmlFor={descId}>Description (optional)</label>
            <HelpHint area="dataset" setting="concept-description" describes={descId} />
          </span>
          <input
            id={descId}
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="bare feet, visible"
          />
        </div>
        <button type="button" className="datasetform__go" onClick={create} disabled={!canCreate}>
          {busy ? "Creating…" : "Create concept"}
        </button>
        {error && <p className="dataset__err">{error}</p>}
      </div>

      <p className="concepts__reminder">{CONCEPT_REMINDER}</p>
    </div>
  );
}
