import { useEffect, useState } from "react";
import "./command-palette.css";

const SHORTCUTS: { keys: string; does: string }[] = [
  { keys: "⌘K / Ctrl+K", does: "Open the command palette" },
  { keys: "?", does: "Show this list" },
  { keys: "Esc", does: "Close any open dialog" },
  { keys: "Enter", does: "Send a chat message (Shift+Enter for a newline)" },
];

const isTypingTarget = (el: EventTarget | null) =>
  el instanceof HTMLElement && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);

/** A "?" overlay listing keyboard shortcuts -- discoverability for the
 *  command palette and anything else that only exists as a key combo. Also
 *  the way to the Help tab for someone who pressed `?` looking for help. */
export function ShortcutsHelp({ onOpenHelp }: { onOpenHelp: () => void }) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "?" && !isTypingTarget(e.target)) {
        e.preventDefault();
        setOpen((o) => !o);
      } else if (e.key === "Escape" && open) {
        setOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!open) return null;

  return (
    // Presentational: a click on the backdrop itself closes; Escape is
    // handled by the window listener above.
    <div
      className="cmdk__backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) setOpen(false);
      }}
    >
      <div
        className="cmdk"
        role="dialog"
        aria-label="Keyboard shortcuts"
        style={{ maxHeight: "none" }}
      >
        <div className="cmdk__group" style={{ padding: "var(--space-4) var(--space-4) 0" }}>
          Keyboard shortcuts
        </div>
        <ul className="cmdk__list">
          {SHORTCUTS.map((s) => (
            <li key={s.keys}>
              <div className="cmdk__item" style={{ cursor: "default" }}>
                <span>{s.does}</span>
                <span className="cmdk__hint">{s.keys}</span>
              </div>
            </li>
          ))}
        </ul>
        <div style={{ padding: "0 var(--space-4) var(--space-4)" }}>
          <button
            type="button"
            className="chip"
            onClick={() => {
              setOpen(false);
              onOpenHelp();
            }}
          >
            Open Help
          </button>
        </div>
      </div>
    </div>
  );
}
