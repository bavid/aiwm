import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useActivePersona, useEffectivePersona, usePersonas } from "../../../lib/hooks";
import {
  createPersona,
  deletePersona,
  setActivePersona,
  setSessionPersona,
  updatePersona,
  type PersonaBody,
  type PersonaMode,
} from "../../../lib/ipc";
import { PersonaChip } from "./PersonaChip";
import { PersonaMenu } from "./PersonaMenu";
import { PersonaManager } from "./PersonaManager";
import "./personas.css";

/** How often the persona list and the global default are re-read while the
 *  menu or the dialog is open — they can only change from in here, so the
 *  poll is really just a backstop for a second window. */
const OPEN_POLL_MS = 5000;
/** Closed, nothing of this is on screen: the interval change re-fetches on
 *  open, which is the only moment the data is needed. */
const IDLE_POLL_MS = 60 * 60 * 1000;

/** The chat header's persona control as a whole: the chip, the menu it opens
 *  and the manage dialog behind it. It owns every persona round-trip so the
 *  three pieces below it stay presentational, and re-reads what it shows after
 *  each one — a delete can move the global default, a session override and the
 *  chip's label at the same time. */
export function PersonaControls({
  sessionId,
  sessionMode,
  sessionPersonaId,
  onSessionsChanged,
}: {
  sessionId: string | null;
  /** This chat's stored override, or `null` while the session list has not
   *  arrived yet — the menu then shows no session choice as taken. */
  sessionMode: PersonaMode | null;
  sessionPersonaId: string | null;
  /** Ask the owner of the session list to re-read it: a persona write can
   *  change this chat's row, and a delete can change any chat's. */
  onSessionsChanged: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [managerOpen, setManagerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chipRef = useRef<HTMLButtonElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const pollMs = menuOpen || managerOpen ? OPEN_POLL_MS : IDLE_POLL_MS;
  const { data: personas, refetch: refetchPersonas } = usePersonas(pollMs);
  const { data: active, refetch: refetchActive } = useActivePersona(pollMs);
  const { data: effective, refetch: refetchEffective } = useEffectivePersona(sessionId);

  const closeMenu = useCallback((returnFocus: boolean) => {
    setMenuOpen(false);
    if (returnFocus) chipRef.current?.focus();
  }, []);

  const openMenu = () => {
    // Last attempt's complaint is about a choice that is now two clicks old.
    setError(null);
    setMenuOpen(true);
  };

  // Clicking anywhere else dismisses the menu -- without stealing focus back
  // to the chip, since the click is already moving it somewhere on purpose.
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: PointerEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [menuOpen]);

  const refreshAll = useCallback(() => {
    refetchPersonas();
    refetchActive();
    refetchEffective();
    onSessionsChanged();
  }, [refetchPersonas, refetchActive, refetchEffective, onSessionsChanged]);

  const pickGlobal = async (personaId: string | null) => {
    setError(null);
    closeMenu(true);
    try {
      const ok = await setActivePersona(personaId);
      if (!ok) setError("That persona no longer exists.");
    } catch (err) {
      setError(messageOf(err));
    }
    refreshAll();
  };

  const pickSession = async (mode: PersonaMode, personaId?: string) => {
    if (!sessionId) return;
    setError(null);
    closeMenu(true);
    try {
      const outcome = await setSessionPersona(sessionId, { mode, persona_id: personaId });
      if (outcome === "unknown_session") setError("This chat no longer exists.");
      else if (outcome === "unknown_persona") setError("That persona no longer exists.");
    } catch (err) {
      setError(messageOf(err));
    }
    refreshAll();
  };

  const openManager = () => {
    setError(null);
    // No focus return: the dialog takes focus itself on mount.
    closeMenu(false);
    setManagerOpen(true);
  };

  const closeManager = useCallback(() => {
    setManagerOpen(false);
    chipRef.current?.focus();
  }, []);

  // The three below deliberately let a rejection through -- including the
  // "it is gone" cases the core reports as `null` / `false` rather than as an
  // error: the dialog shows all of them in the same place, verbatim.
  const handleCreate = async (body: PersonaBody) => {
    await createPersona(body);
    refreshAll();
  };

  const handleUpdate = async (id: string, body: PersonaBody) => {
    const updated = await updatePersona(id, body);
    refreshAll();
    if (!updated) throw new Error(GONE);
  };

  const handleDelete = async (id: string) => {
    const deleted = await deletePersona(id);
    refreshAll();
    if (!deleted) throw new Error(GONE);
  };

  return (
    <div className="persona-controls" ref={wrapRef}>
      <PersonaChip
        ref={chipRef}
        effective={effective}
        optedOut={sessionMode === "none"}
        open={menuOpen}
        menuId={menuId}
        onToggle={() => (menuOpen ? closeMenu(false) : openMenu())}
      />
      {menuOpen && (
        <PersonaMenu
          id={menuId}
          personas={personas ?? []}
          activeId={active?.id ?? null}
          sessionId={sessionId}
          sessionMode={sessionMode}
          sessionPersonaId={sessionPersonaId}
          onPickGlobal={pickGlobal}
          onPickSession={pickSession}
          onManage={openManager}
          onClose={() => closeMenu(true)}
        />
      )}
      {error && (
        <p className="persona-controls__err" role="alert">
          {error}
        </p>
      )}
      {managerOpen && (
        <PersonaManager
          personas={personas ?? []}
          onCreate={handleCreate}
          onUpdate={handleUpdate}
          onDelete={handleDelete}
          onClose={closeManager}
        />
      )}
    </div>
  );
}

const GONE = "That persona no longer exists.";

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
