import { useRef, useState } from "react";
import {
  createSession,
  deleteSession,
  renameSession,
  setSessionArchived,
  type Session,
  type SessionCapability,
} from "../lib/ipc";
import "./session-sidebar.css";

interface SessionRow {
  id: string;
  name: string;
  archived_at: string | null;
}

/** The words one capability puts on the shared list. A session groups chats
 *  on Chat, images on Image and clips on Video, so only the wording differs —
 *  never the layout or the actions. */
export interface SessionSidebarLabels {
  /** The create button, e.g. "+ New chat". */
  create: string;
  /** The name a freshly created session starts with, e.g. "New chat". */
  createdName: string;
  /** Shown while there is no unarchived session yet. */
  empty: string;
  /** The confirm text Delete asks, for the session called `name`. */
  confirmDelete: (name: string) => string;
  /** The bottom link that selects "no session at all". */
  unsorted: string;
}

/** ChatGPT/Claude-style session list docked to the left of a tab — the Chat,
 *  Image and Video tabs all dock this same list, so switching between them
 *  needs no relearning. "New …" always creates a real, named session, so
 *  there's no more unlabeled "Ungrouped" limbo to land in; existing history
 *  with no session stays reachable through a small, deliberately minor link
 *  at the bottom rather than a prominent group. */
export function SessionSidebar({
  capability,
  labels,
  activeId,
  onChange,
  sessions,
  onRefetch,
  nameOnCreate = false,
}: {
  capability: SessionCapability;
  labels: SessionSidebarLabels;
  activeId: string | null;
  onChange: (id: string | null) => void;
  /** The sessions of this capability, polled by the hosting tab: Chat's
   *  persona chip needs the active session's own row too, and one poll serves
   *  both. `null` = not read yet. */
  sessions: Session[] | null;
  onRefetch: () => void;
  /** Open the new row's name field right away. Chat leaves this off because
   *  a chat's first message renames it by itself; Image and Video have no
   *  such source, so there naming it on the spot beats a list of rows all
   *  called "New session". */
  nameOnCreate?: boolean;
}) {
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [showArchived, setShowArchived] = useState(false);
  // Enter commits directly and blur also commits (click-away-to-save); this
  // guards against both firing for the same edit -- Enter moves focus off the
  // input, which would otherwise blur-commit a second time.
  const settled = useRef(false);

  const active = (sessions ?? []).filter((s) => !s.archived_at);
  const archived = (sessions ?? []).filter((s) => s.archived_at);

  const startRename = (id: string, name: string) => {
    settled.current = false;
    setRenamingId(id);
    setDraftName(name);
  };

  const create = async () => {
    const session = await createSession({ capability, name: labels.createdName });
    onRefetch();
    onChange(session.id);
    if (nameOnCreate) startRename(session.id, session.name);
  };

  const commitRename = async () => {
    if (settled.current) return;
    settled.current = true;
    const id = renamingId;
    const name = draftName.trim();
    setRenamingId(null);
    if (!id || !name) return;
    await renameSession(id, name);
    onRefetch();
  };

  const cancelRename = () => {
    settled.current = true;
    setRenamingId(null);
  };

  const remove = async (id: string, name: string) => {
    if (!window.confirm(labels.confirmDelete(name))) return;
    await deleteSession(id);
    onRefetch();
    if (activeId === id) onChange(null);
  };

  const setArchived = async (id: string, archived: boolean) => {
    await setSessionArchived(id, archived);
    onRefetch();
    if (archived && activeId === id) onChange(null);
  };

  const row = (s: SessionRow) => (
    <div
      key={s.id}
      className={
        s.id === activeId
          ? "session-sidebar__row session-sidebar__row--active"
          : "session-sidebar__row"
      }
    >
      {renamingId === s.id ? (
        <input
          autoFocus
          className="session-sidebar__rename-input"
          aria-label={`Rename ${s.name}`}
          value={draftName}
          onChange={(e) => setDraftName(e.target.value)}
          onBlur={commitRename}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitRename();
            if (e.key === "Escape") cancelRename();
          }}
          spellCheck={false}
        />
      ) : (
        <>
          {/* `aria-current` as well as the accent colour: the active row must
              not be signalled by colour alone. */}
          <button
            type="button"
            className="session-sidebar__select"
            aria-current={s.id === activeId ? "true" : undefined}
            onClick={() => onChange(s.id)}
          >
            {s.name}
          </button>
          <button
            type="button"
            className="session-sidebar__icon"
            aria-label={`Rename ${s.name}`}
            onClick={() => startRename(s.id, s.name)}
          >
            ✎
          </button>
          {s.archived_at ? (
            <button
              type="button"
              className="session-sidebar__icon"
              aria-label={`Unarchive ${s.name}`}
              onClick={() => setArchived(s.id, false)}
            >
              ⤴
            </button>
          ) : (
            <button
              type="button"
              className="session-sidebar__icon"
              aria-label={`Archive ${s.name}`}
              onClick={() => setArchived(s.id, true)}
            >
              ⤓
            </button>
          )}
          <button
            type="button"
            className="session-sidebar__icon session-sidebar__icon--danger"
            aria-label={`Delete ${s.name}`}
            onClick={() => remove(s.id, s.name)}
          >
            ×
          </button>
        </>
      )}
    </div>
  );

  return (
    <aside className="session-sidebar">
      <button type="button" className="session-sidebar__new" onClick={create}>
        {labels.create}
      </button>
      <div className="session-sidebar__list">
        {active.length === 0 && <p className="muted">{labels.empty}</p>}
        {active.map(row)}
      </div>
      {archived.length > 0 && (
        <>
          <button
            type="button"
            className="session-sidebar__archived-toggle"
            onClick={() => setShowArchived((v) => !v)}
          >
            {showArchived ? "Hide" : "Show"} archived ({archived.length})
          </button>
          {showArchived && <div className="session-sidebar__list">{archived.map(row)}</div>}
        </>
      )}
      <div className="session-sidebar__unsorted">
        <button type="button" aria-pressed={activeId === null} onClick={() => onChange(null)}>
          {labels.unsorted}
        </button>
      </div>
    </aside>
  );
}
