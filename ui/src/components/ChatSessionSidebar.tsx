import { SessionSidebar, type SessionSidebarLabels } from "./SessionSidebar";
import type { Session } from "../lib/ipc";
import "./chat-session-sidebar.css";

const LABELS: SessionSidebarLabels = {
  create: "+ New chat",
  createdName: "New chat",
  empty: "No chats yet.",
  confirmDelete: (name) => `Delete “${name}”?\n\nIts messages stay in your history, just unsorted.`,
  unsorted: "Unsorted messages",
};

/** The Chat tab's wording on the shared {@link SessionSidebar} — Image and
 *  Video dock the same list with their own nouns. A chat's first message
 *  renames it, so a new one is not put straight into its name field. */
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
  return (
    <SessionSidebar
      capability="chat"
      labels={LABELS}
      activeId={activeId}
      onChange={onChange}
      sessions={sessions}
      onRefetch={onRefetch}
    />
  );
}
