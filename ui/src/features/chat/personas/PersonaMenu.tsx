import { useEffect, useId, useRef } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { Persona, PersonaMode } from "../../../lib/ipc";

/** How much of the window's edge the open menu keeps clear of. */
const EDGE_GAP_PX = 8;

/** What the chip opens: the global default on top, this chat's override below
 *  it, and the way into the manage dialog. Both halves are shown at once
 *  rather than behind a toggle — the whole point of the feature is that the
 *  two settings differ, and hiding one makes the chip's origin label a
 *  riddle. */
export function PersonaMenu({
  id,
  personas,
  activeId,
  sessionId,
  sessionMode,
  sessionPersonaId,
  onPickGlobal,
  onPickSession,
  onManage,
  onClose,
}: {
  id: string;
  personas: Persona[];
  /** The globally active persona, `null` = none. */
  activeId: string | null;
  /** `null` = "Ungrouped", where only the global half applies. */
  sessionId: string | null;
  sessionMode: PersonaMode;
  sessionPersonaId: string | null;
  onPickGlobal: (personaId: string | null) => void;
  onPickSession: (mode: PersonaMode, personaId?: string) => void;
  onManage: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const noteId = useId();

  // Opening a menu moves focus into it -- otherwise the arrow keys below have
  // nothing to move from, and a keyboard user would have to tab past the whole
  // header to reach the first choice.
  useEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    itemsOf(el)[0]?.focus();
    // The menu hangs off the chip's leading edge, which is fine until the
    // window gets narrow enough that the chip itself is near the right edge --
    // then the popup would run off-screen, where it cannot be clicked. Nudge
    // it back in by exactly as much as it overhangs.
    const overhang = el.getBoundingClientRect().right - (window.innerWidth - EDGE_GAP_PX);
    if (overhang > 0) el.style.transform = `translateX(${-Math.round(overhang)}px)`;
  }, []);

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    // Tab leaves the menu on purpose; close it, but let the browser move
    // focus as it normally would rather than trapping it (this is a menu,
    // not a modal).
    if (e.key === "Tab") {
      onClose();
      return;
    }
    const items = itemsOf(menuRef.current);
    if (items.length === 0) return;
    const here = items.indexOf(document.activeElement as HTMLElement);
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (step !== 0) {
      e.preventDefault();
      const next = here < 0 ? 0 : (here + step + items.length) % items.length;
      items[next].focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      items[0].focus();
    } else if (e.key === "End") {
      e.preventDefault();
      items[items.length - 1].focus();
    }
  };

  return (
    <div
      id={id}
      ref={menuRef}
      className="persona-menu"
      role="menu"
      aria-label="Persona"
      aria-describedby={sessionId ? undefined : noteId}
      onKeyDown={onKeyDown}
    >
      <div className="persona-menu__section" role="group" aria-label="Global default">
        <p className="persona-menu__group" aria-hidden="true">
          Global default
        </p>
        <MenuChoice
          icon="○"
          label="None"
          checked={activeId === null}
          onSelect={() => onPickGlobal(null)}
        />
        {personas.map((p) => (
          <MenuChoice
            key={p.id}
            icon={p.icon}
            label={p.name}
            checked={activeId === p.id}
            onSelect={() => onPickGlobal(p.id)}
          />
        ))}
      </div>

      {sessionId ? (
        <div className="persona-menu__section" role="group" aria-label="This chat">
          <p className="persona-menu__group" aria-hidden="true">
            This chat
          </p>
          <MenuChoice
            icon="↳"
            label="Inherit global"
            checked={sessionMode === "inherit"}
            onSelect={() => onPickSession("inherit")}
          />
          <MenuChoice
            icon="○"
            label="No persona for this chat"
            checked={sessionMode === "none"}
            onSelect={() => onPickSession("none")}
          />
          {personas.map((p) => (
            <MenuChoice
              key={p.id}
              icon={p.icon}
              label={p.name}
              checked={sessionMode === "persona" && sessionPersonaId === p.id}
              onSelect={() => onPickSession("persona", p.id)}
            />
          ))}
        </div>
      ) : (
        <p className="persona-menu__note" id={noteId}>
          This chat is ungrouped, so the global default applies. Pick a chat in the sidebar to give
          it its own persona.
        </p>
      )}

      <div className="persona-menu__sep" role="separator" />
      <button
        type="button"
        role="menuitem"
        data-menuitem=""
        className="persona-menu__item persona-menu__manage"
        onClick={onManage}
      >
        Manage personas…
      </button>
    </div>
  );
}

function MenuChoice({
  icon,
  label,
  checked,
  onSelect,
}: {
  icon: string;
  label: string;
  checked: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitemradio"
      data-menuitem=""
      aria-checked={checked}
      className="persona-menu__item"
      onClick={onSelect}
    >
      <span className="persona-menu__tick" aria-hidden="true">
        {checked ? "✓" : ""}
      </span>
      <span className="persona-menu__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="persona-menu__label">{label}</span>
    </button>
  );
}

/** The menu's focusable rows, in the order they are on screen. Read from the
 *  DOM rather than kept as an array of refs: the rows come from three sources
 *  (fixed choices, one per persona, the manage row) and the list changes while
 *  the menu is open. */
function itemsOf(menu: HTMLDivElement | null): HTMLElement[] {
  if (!menu) return [];
  return [...menu.querySelectorAll<HTMLElement>("[data-menuitem]")];
}
