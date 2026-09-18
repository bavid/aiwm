import type { Ref } from "react";
import type { EffectivePersona } from "../../../lib/ipc";

/** The chat header's persona control: icon + name of the persona that will
 *  actually answer, and where that came from ("global" / "this chat").
 *  Presentational — it opens the menu and says nothing about how the choice is
 *  made (see `PersonaControls`). */
export function PersonaChip({
  effective,
  optedOut,
  open,
  menuId,
  onToggle,
  ref,
}: {
  /** `null` while the first resolve is still in flight. */
  effective: EffectivePersona | null;
  /** This chat set "No persona for this chat" — the core reports that as
   *  origin `none` just like "nothing is set", so only the session's own mode
   *  can tell the two apart. */
  optedOut: boolean;
  open: boolean;
  menuId: string;
  onToggle: () => void;
  ref?: Ref<HTMLButtonElement>;
}) {
  const persona = effective?.persona ?? null;
  const origin = originLabel(effective, optedOut);
  // Not yet resolved is not the same as "none": saying "No persona" here would
  // be a wrong answer for the moment after a session switch, when the chip is
  // remounted and the first resolve is still in flight.
  const name = effective ? (persona ? persona.name : "No persona") : "Checking…";

  return (
    <button
      ref={ref}
      type="button"
      className="persona-chip"
      data-active={persona ? "yes" : "no"}
      aria-haspopup="menu"
      aria-expanded={open}
      aria-controls={open ? menuId : undefined}
      aria-label={`Persona: ${name}${origin ? `, ${origin}` : ""} — choose a persona`}
      onClick={onToggle}
    >
      <span className="persona-chip__icon" aria-hidden="true">
        {persona ? persona.icon : "○"}
      </span>
      <span className="persona-chip__name">{name}</span>
      {origin && <span className="persona-chip__origin">{origin}</span>}
    </button>
  );
}

function originLabel(effective: EffectivePersona | null, optedOut: boolean): string | null {
  if (!effective) return null;
  if (effective.origin === "session") return "this chat";
  if (effective.origin === "global") return "global";
  return optedOut ? "this chat" : null;
}
