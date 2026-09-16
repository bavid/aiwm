import { useId, useMemo, useState, type KeyboardEvent, type MouseEvent } from "react";
import {
  assignConcept,
  createConcept,
  unassignConcept,
  type DatasetConcept,
  type DatasetFrame,
} from "../../lib/ipc";
import { CONCEPT_REMINDER, ConceptRow } from "./ConceptRow";
import { fileName } from "./format";
import { buildSets, type SetGrouping } from "./sets";
import { tokenWarning } from "./tokens";

/** Keys pressed inside a field belong to that field, not to the set. */
function isFieldTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

type Props = {
  datasetId: string;
  frames: DatasetFrame[];
  concepts: DatasetConcept[];
  /** frame id -> concept ids, as `useFrameConceptMap` returns it. */
  conceptMap: Record<string, string[]>;
  imageUrlFor: (frame: DatasetFrame) => string;
  /** Refetch concepts + the frame/concept map after a write. */
  onChanged: () => void;
};

/** Guided curation: walk the kept frames one bounded set at a time, select
 *  what belongs to a concept and attach the whole batch in one go. The
 *  right-hand summary keeps the per-concept budget in view while you work. */
export function LearnSets({
  datasetId,
  frames,
  concepts,
  conceptMap,
  imageUrlFor,
  onChanged,
}: Props) {
  const [grouping, setGrouping] = useState<SetGrouping>("clip");
  const [setIndex, setSetIndex] = useState(0);
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set());
  /** Index within the current set that a Shift+click ranges back to. */
  const [anchor, setAnchor] = useState<number | null>(null);
  const [conceptId, setConceptId] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [isCreating, setIsCreating] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [draftToken, setDraftToken] = useState("");
  const [draftDesc, setDraftDesc] = useState("");
  const [isBusy, setIsBusy] = useState(false);

  const groupingId = useId();
  const conceptSelectId = useId();
  const nameId = useId();
  const tokenId = useId();
  const descId = useId();

  const sets = useMemo(() => buildSets(frames, grouping), [frames, grouping]);
  // Clamped during render: the poll can shrink the set list under a curator
  // who is sitting on the last set, and an effect would flash an empty view.
  const index = sets.length === 0 ? 0 : Math.min(setIndex, sets.length - 1);
  const current = sets[index] ?? [];

  const tokenById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const c of concepts) map[c.id] = c.token;
    return map;
  }, [concepts]);

  const tokensFor = (frameId: string): string[] =>
    (conceptMap[frameId] ?? []).map((id) => tokenById[id]).filter((t): t is string => !!t);

  const draftWarning = draftToken.trim() === "" ? null : tokenWarning(draftToken);
  const canCreate = draftName.trim() !== "" && draftToken.trim() !== "" && !isBusy;

  const goTo = (next: number) => {
    if (sets.length === 0) return;
    const clamped = Math.min(Math.max(next, 0), sets.length - 1);
    setSetIndex(clamped);
    setAnchor(null);
  };

  const changeGrouping = (next: SetGrouping) => {
    setGrouping(next);
    setSetIndex(0);
    setAnchor(null);
    setSelected(new Set());
    setStatus(null);
  };

  const clickThumb = (position: number, withShift: boolean) => {
    setSelected((cur) => {
      const next = new Set(cur);
      if (withShift && anchor !== null) {
        const from = Math.min(anchor, position);
        const to = Math.max(anchor, position);
        for (let i = from; i <= to; i += 1) {
          const frame = current[i];
          if (frame) next.add(frame.id);
        }
        return next;
      }
      const frame = current[position];
      if (!frame) return next;
      if (!next.delete(frame.id)) next.add(frame.id);
      return next;
    });
    if (!withShift) setAnchor(position);
  };

  const selectAll = () => setSelected(new Set(current.map((f) => f.id)));

  const invert = () =>
    setSelected((cur) => {
      const next = new Set(cur);
      for (const frame of current) {
        if (!next.delete(frame.id)) next.add(frame.id);
      }
      return next;
    });

  const clear = () => {
    setSelected(new Set());
    setAnchor(null);
  };

  const apply = async (attach: boolean) => {
    if (!conceptId || selected.size === 0) return;
    const ids = [...selected];
    setError(null);
    try {
      if (attach) {
        const summary = await assignConcept(conceptId, ids);
        setStatus(`${summary.attached} of ${summary.requested} assigned`);
      } else {
        await unassignConcept(conceptId, ids);
        setStatus(`${ids.length} removed`);
      }
      onChanged();
    } catch (e) {
      setStatus(null);
      setError(String(e));
    }
  };

  const create = async () => {
    if (!canCreate) return;
    setIsBusy(true);
    setError(null);
    try {
      const row = await createConcept(datasetId, {
        name: draftName.trim(),
        token: draftToken.trim(),
        description: draftDesc.trim() || undefined,
      });
      // Preselect it: the curator almost always assigns the selection they
      // were looking at when they reached for "+ New concept".
      setConceptId(row.id);
      setDraftName("");
      setDraftToken("");
      setDraftDesc("");
      setIsCreating(false);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setIsBusy(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (isFieldTarget(event.target)) return;
    switch (event.key) {
      case "ArrowLeft":
        event.preventDefault();
        goTo(index - 1);
        return;
      case "ArrowRight":
        event.preventDefault();
        goTo(index + 1);
        return;
      case "a":
      case "A":
        event.preventDefault();
        selectAll();
        return;
      case "i":
      case "I":
        event.preventDefault();
        invert();
        return;
      case "Escape":
        event.preventDefault();
        clear();
        return;
      case "Enter": {
        // A focused button already answers Enter with its own click.
        const tag = event.target instanceof HTMLElement ? event.target.tagName : "";
        if (tag === "BUTTON" || tag === "A") return;
        event.preventDefault();
        apply(true);
        return;
      }
      default:
    }
  };

  const conceptForm = (
    <div className="learn__new">
      {isCreating ? (
        <>
          <label className="datasetform__field" htmlFor={nameId}>
            <span>Name</span>
            <input
              id={nameId}
              type="text"
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
              placeholder="Kenji"
            />
          </label>
          <label className="datasetform__field" htmlFor={tokenId}>
            <span>Token</span>
            <input
              id={tokenId}
              type="text"
              value={draftToken}
              onChange={(e) => setDraftToken(e.target.value)}
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
              value={draftDesc}
              onChange={(e) => setDraftDesc(e.target.value)}
              placeholder="bare feet, visible"
            />
          </label>
          <div className="learn__new-actions">
            <button type="button" className="chip" onClick={create} disabled={!canCreate}>
              {isBusy ? "Creating…" : "Create"}
            </button>
            <button type="button" className="chip" onClick={() => setIsCreating(false)}>
              Cancel
            </button>
          </div>
        </>
      ) : (
        <button type="button" className="chip" onClick={() => setIsCreating(true)}>
          + New concept
        </button>
      )}
    </div>
  );

  return (
    // Focusable so the set shortcuts have somewhere to land; every action is
    // also reachable as a real button below.
    <div
      className="card learn"
      tabIndex={0}
      onKeyDown={onKeyDown}
      role="group"
      aria-label="Guided concept sets"
    >
      <div className="learn__main">
        <div className="learn__header">
          <div>
            <h3 className="learn__title">
              {sets.length === 0
                ? "No sets yet"
                : `Set ${index + 1} / ${sets.length} · ${current.length} frames`}
            </h3>
            {current.length > 0 && (
              <p className="learn__source">{fileName(current[0].source_path)}</p>
            )}
          </div>
          <label className="datasetform__field datasetform__field--inline" htmlFor={groupingId}>
            <span>Group</span>
            <select
              id={groupingId}
              value={grouping}
              onChange={(e) => changeGrouping(e.target.value as SetGrouping)}
            >
              <option value="clip">By clip</option>
              <option value="similarity">By similarity (coarse)</option>
            </select>
          </label>
        </div>

        <p className="learn__hint">
          Click to pick, Shift+click for a range. ←/→ change set, A selects all, I inverts, Esc
          clears, Enter assigns.
        </p>

        {sets.length === 0 ? (
          <p className="learn__empty">
            No kept frames yet — run a prep job or restore frames in the grid.
          </p>
        ) : (
          <>
            <div className="learn__grid">
              {current.map((frame, position) => {
                const isSelected = selected.has(frame.id);
                const tokens = tokensFor(frame.id);
                return (
                  <button
                    key={frame.id}
                    type="button"
                    className="learn__thumb"
                    aria-pressed={isSelected}
                    // The thumbnail's own name: the chips stacked on top of it
                    // would otherwise be read as part of the button's label.
                    aria-label={frame.caption || frame.tag}
                    onClick={(e: MouseEvent<HTMLButtonElement>) =>
                      clickThumb(position, e.shiftKey)
                    }
                  >
                    <img
                      src={imageUrlFor(frame)}
                      alt={frame.caption || frame.tag}
                      loading="lazy"
                    />
                    <span className="learn__thumb-tag">{frame.tag}</span>
                    {tokens.length > 0 && (
                      <span className="learn__thumb-concepts">
                        {tokens.map((token) => (
                          <span key={token} className="framecard__concept">
                            {token}
                          </span>
                        ))}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>

            <div className="learn__actions">
              <span className="dataset__toolbar-count">{selected.size} selected</span>
              <button type="button" className="chip" onClick={selectAll}>
                All in set
              </button>
              <button type="button" className="chip" onClick={invert}>
                Invert selection
              </button>
              <button type="button" className="chip" onClick={clear}>
                Clear
              </button>
              <label
                className="datasetform__field datasetform__field--inline"
                htmlFor={conceptSelectId}
              >
                <span className="dataset__toolbar-label">Concept</span>
                <select
                  id={conceptSelectId}
                  value={conceptId}
                  onChange={(e) => setConceptId(e.target.value)}
                >
                  <option value="">Choose a concept…</option>
                  {concepts.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name} ({c.token})
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                className="chip"
                disabled={!conceptId || selected.size === 0}
                onClick={() => apply(true)}
              >
                Assign
              </button>
              <button
                type="button"
                className="chip"
                disabled={!conceptId || selected.size === 0}
                onClick={() => apply(false)}
              >
                Remove
              </button>
            </div>

            <div className="learn__nav">
              <button type="button" className="chip" disabled={index === 0} onClick={() => goTo(index - 1)}>
                ← Previous
              </button>
              <button
                type="button"
                className="chip"
                disabled={index >= sets.length - 1}
                onClick={() => goTo(index + 1)}
              >
                Next →
              </button>
            </div>
          </>
        )}

        {status && <p className="learn__status">{status}</p>}
        {error && <p className="dataset__err">{error}</p>}
      </div>

      <aside className="learn__summary" aria-label="Concept summary">
        <h4 className="learn__summary-title">Concepts</h4>
        {concepts.length === 0 ? (
          <p className="learn__empty">
            No concepts yet — create one for the first thing this dataset should teach, then assign
            frames to it.
          </p>
        ) : (
          <ul className="concepts__list">
            {concepts.map((c) => (
              <ConceptRow key={c.id} concept={c} />
            ))}
          </ul>
        )}
        {conceptForm}
        <p className="concepts__reminder">{CONCEPT_REMINDER}</p>
      </aside>
    </div>
  );
}
