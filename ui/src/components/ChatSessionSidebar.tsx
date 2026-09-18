import { useRef, useState } from "react";
import {
  createSession,
  deleteSession,
  renameSession,
  setSessionArchived,
  type Session,
} from "../lib/ipc";
import "./chat-session-sidebar.css";

interface SessionRow {
  id: string;
  name: string;
  archived_at: string | null;
}

/** ChatGPT/Claude-style session list docked to the left of the Chat tab —
 *  replaces the old header dropdown. "New chat" always creates a real, named
 *  session, so there's no more unlabeled "Ungrouped" limbo to land in;
 *  existing history with no session stays reachable through a small,
 *  deliberately minor link at the bottom rather than a prominent group. */
export function ChatSessionSidebar({
  activeId,
  onChange,
  sessions,
  onRefetch,
}: {
  activeId: string | null;
  onChange: (id: string | null) => void;
  /** The chat sessions, polled by the Chat tab: the persona chip needs the
   *  active session's own row too, and one poll serves both. `null` = not read
   *  yet. */
  sessions: Session[] | null;
  onRefetch: () => void;
}) {
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [showArchived, setShowArchived] = useState(false);
  // Same Enter-commits / blur-also-commits guard as SessionSwitcher.
  const settled = useRef(false);

  const active = (sessions ?? []).filter((s) => !s.archived_at);
  const archived = (sessions ?? []).filter((s) => s.archived_at);

  const newChat = async () => {
    const session = await createSession({ capability: "chat", name: "New chat" });
    onRefetch();
    onChange(session.id);
  };

  const startRename = (id: string, name: string) => {
    settled.current = false;
    setRenamingId(id);
    setDraftName(name);
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
    if (
      !window.confirm(`Delete “${name}”?\n\nIts messages stay in your history, just unsorted.`)
    ) {
      return;
    }
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
        s.id === activeId ? "chat-sidebar__row chat-sidebar__row--active" : "chat-sidebar__row"
      }
    >
      {renamingId === s.id ? (
        <input
          autoFocus
          className="chat-sidebar__rename-input"
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
          <button type="button" className="chat-sidebar__select" onClick={() => onChange(s.id)}>
            {s.name}
          </button>
          <button
            type="button"
            className="chat-sidebar__icon"
            title="Rename"
            onClick={() => startRename(s.id, s.name)}
          >
            ✎
          </button>
          {s.archived_at ? (
            <button
              type="button"
              className="chat-sidebar__icon"
              title="Unarchive"
              onClick={() => setArchived(s.id, false)}
            >
              ⤴
            </button>
          ) : (
            <button
              type="button"
              className="chat-sidebar__icon"
              title="Archive"
              onClick={() => setArchived(s.id, true)}
            >
              ⤓
            </button>
          )}
          <button
            type="button"
            className="chat-sidebar__icon chat-sidebar__icon--danger"
            title="Delete"
            onClick={() => remove(s.id, s.name)}
          >
            ×
          </button>
        </>
      )}
    </div>
  );

  return (
    <aside className="chat-sidebar">
      <button type="button" className="chat-sidebar__new" onClick={newChat}>
        + New chat
      </button>
      <div className="chat-sidebar__list">
        {active.length === 0 && <p className="muted">No chats yet.</p>}
        {active.map(row)}
      </div>
      {archived.length > 0 && (
        <>
          <button
            type="button"
            className="chat-sidebar__archived-toggle"
            onClick={() => setShowArchived((v) => !v)}
          >
            {showArchived ? "Hide" : "Show"} archived ({archived.length})
          </button>
          {showArchived && <div className="chat-sidebar__list">{archived.map(row)}</div>}
        </>
      )}
      <div className="chat-sidebar__unsorted">
        <button type="button" aria-pressed={activeId === null} onClick={() => onChange(null)}>
          Unsorted messages
        </button>
      </div>
    </aside>
  );
}
