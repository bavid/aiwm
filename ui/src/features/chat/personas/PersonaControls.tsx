import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useActivePersona, useEffectivePersona, usePersonas, useSessions } from "../../../lib/hooks";
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

/** The chat header's persona control as a whole: the chip, the menu it opens
 *  and the manage dialog behind it. It owns every persona round-trip so the
 *  three pieces below it stay presentational, and re-reads all four queries
 *  after each one — a delete can move the global default, a session override
 *  and the chip's label at the same time. */
export function PersonaControls({ sessionId }: { sessionId: string | null }) {
  const { data: personas, refetch: refetchPersonas } = usePersonas();
  const { data: active, refetch: refetchActive } = useActivePersona();
  const { data: effective, refetch: refetchEffective } = useEffectivePersona(sessionId);
  const { data: sessions, refetch: refetchSessions } = useSessions("chat");

  const [menuOpen, setMenuOpen] = useState(false);
  const [managerOpen, setManagerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chipRef = useRef<HTMLButtonElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const session = sessions?.find((s) => s.id === sessionId) ?? null;
  const sessionMode: PersonaMode = session?.persona_mode ?? "inherit";

  const closeMenu = useCallback((returnFocus: boolean) => {
    setMenuOpen(false);
    if (returnFocus) chipRef.current?.focus();
  }, []);

  // Switching chats while the menu is open would leave it showing the old
  // chat's override.
  useEffect(() => {
    setMenuOpen(false);
  }, [sessionId]);

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
    refetchSessions();
  }, [refetchPersonas, refetchActive, refetchEffective, refetchSessions]);

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

  const closeManager = () => {
    setManagerOpen(false);
    chipRef.current?.focus();
  };

  // The three below deliberately let a rejection through: the dialog shows the
  // core's message verbatim next to the field that caused it.
  const handleCreate = async (body: PersonaBody) => {
    await createPersona(body);
    refreshAll();
  };

  const handleUpdate = async (id: string, body: PersonaBody) => {
    await updatePersona(id, body);
    refreshAll();
  };

  const handleDelete = async (id: string) => {
    await deletePersona(id);
    refreshAll();
  };

  return (
    <div className="persona-controls" ref={wrapRef}>
      <PersonaChip
        ref={chipRef}
        effective={effective}
        optedOut={sessionMode === "none"}
        open={menuOpen}
        menuId={menuId}
        onToggle={() => setMenuOpen((o) => !o)}
      />
      {menuOpen && (
        <PersonaMenu
          id={menuId}
          personas={personas ?? []}
          activeId={active?.id ?? null}
          sessionId={sessionId}
          sessionMode={sessionMode}
          sessionPersonaId={session?.persona_id ?? null}
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

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
