import { useRef, useState } from "react";
import { useSessions } from "../lib/hooks";
import {
  createSession,
  deleteSession,
  renameSession,
  setSessionArchived,
  type SessionCapability,
} from "../lib/ipc";
import "./session-switcher.css";

interface SessionSwitcherProps {
  capability: SessionCapability;
  activeId: string | null;
  onChange: (id: string | null) => void;
}

/** Picks which named "project" (session) new jobs for this capability get
 *  grouped under. Shared by Chat/Image/Video — each tab keeps its own active
 *  session in local state and passes it here. */
export function SessionSwitcher({ capability, activeId, onChange }: SessionSwitcherProps) {
  const { data: sessions, refetch } = useSessions(capability);
  const [creating, setCreating] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [renamingId, setRenamingId] = useState<string | null>(null);
  // Enter commits directly and blur also commits (click-away-to-save); this
  // guards against both firing for the same edit -- Enter moves focus off the
  // input, which would otherwise blur-commit a second time.
  const settled = useRef(false);

  const active = (sessions ?? []).filter((s) => !s.archived_at);
  const archived = (sessions ?? []).filter((s) => s.archived_at);
  const current = (sessions ?? []).find((s) => s.id === activeId) ?? null;

  const startCreate = () => {
    settled.current = false;
    setCreating(true);
    setDraftName("");
  };

  const commitCreate = async () => {
    if (settled.current) return;
    settled.current = true;
    const name = draftName.trim();
    setCreating(false);
    if (!name) return;
    const session = await createSession({ capability, name });
    refetch();
    onChange(session.id);
  };

  const cancelCreate = () => {
    settled.current = true;
    setCreating(false);
  };

  const startRename = () => {
    if (!current) return;
    settled.current = false;
    setRenamingId(current.id);
    setDraftName(current.name);
  };

  const commitRename = async () => {
    if (settled.current) return;
    settled.current = true;
    const id = renamingId;
    const name = draftName.trim();
    setRenamingId(null);
    if (!id || !name) return;
    await renameSession(id, name);
    refetch();
  };

  const cancelRename = () => {
    settled.current = true;
    setRenamingId(null);
  };

  const archiveCurrent = async () => {
    if (!current) return;
    await setSessionArchived(current.id, true);
    refetch();
    onChange(null);
  };

  const deleteCurrent = async () => {
    if (!current) return;
    if (
      !window.confirm(
        `Delete session “${current.name}”?\n\nIts jobs stay in your history, just ungrouped.`,
      )
    )
      return;
    await deleteSession(current.id);
    refetch();
    onChange(null);
  };

  if (creating) {
    return (
      <div className="session-switcher session-switcher--editing">
        <input
          autoFocus
          value={draftName}
          onChange={(e) => setDraftName(e.target.value)}
          onBlur={commitCreate}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitCreate();
            if (e.key === "Escape") cancelCreate();
          }}
          placeholder="Session name"
          spellCheck={false}
        />
      </div>
    );
  }

  if (renamingId) {
    return (
      <div className="session-switcher session-switcher--editing">
        <input
          autoFocus
          value={draftName}
          onChange={(e) => setDraftName(e.target.value)}
          onBlur={commitRename}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitRename();
            if (e.key === "Escape") cancelRename();
          }}
          spellCheck={false}
        />
      </div>
    );
  }

  return (
    <div className="session-switcher">
      <select
        aria-label="Session"
        value={activeId ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
      >
        <option value="">Ungrouped</option>
        {active.map((s) => (
          <option key={s.id} value={s.id}>
            {s.name}
          </option>
        ))}
        {archived.length > 0 && (
          <optgroup label="Archived">
            {archived.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </optgroup>
        )}
      </select>
      <button type="button" className="session-switcher__icon" onClick={startCreate} aria-label="New session">
        +
      </button>
      {current && (
        <>
          <button
            type="button"
            className="session-switcher__icon"
            onClick={startRename}
            aria-label={`Rename session ${current.name}`}
          >
            ✎
          </button>
          {current.archived_at ? (
            <button
              type="button"
              className="session-switcher__icon"
              onClick={() => setSessionArchived(current.id, false).then(refetch)}
              aria-label={`Unarchive session ${current.name}`}
            >
              ⤴
            </button>
          ) : (
            <button
              type="button"
              className="session-switcher__icon"
              onClick={archiveCurrent}
              aria-label={`Archive session ${current.name}`}
            >
              ⤓
            </button>
          )}
          <button
            type="button"
            className="session-switcher__icon session-switcher__icon--danger"
            onClick={deleteCurrent}
            aria-label={`Delete session ${current.name}`}
          >
            ×
          </button>
        </>
      )}
    </div>
  );
}
