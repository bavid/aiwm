import { useEffect, useId, useRef, useState } from "react";
import type { Persona, PersonaBody } from "../../../lib/ipc";
import { PERSONA_TEMPLATES } from "./persona-templates";

/** The core's own limits (`core::persona::validate`), mirrored here so the
 *  form can say what is wrong before a round-trip. The server still decides:
 *  whatever it rejects is shown verbatim next to the Save button. */
const NAME_MAX_CHARS = 60;
const ICON_MAX_BYTES = 16;
const PROMPT_MAX_CHARS = 8000;

const EMPTY: PersonaBody = { name: "", icon: "", system_prompt: "" };

type FieldErrors = { name?: string; icon?: string; system_prompt?: string };

/** Create, edit and delete personas. A modal because it is a detour from the
 *  conversation, not part of it — Esc and the backdrop both get you back. */
export function PersonaManager({
  personas,
  onCreate,
  onUpdate,
  onDelete,
  onClose,
}: {
  personas: Persona[];
  onCreate: (body: PersonaBody) => Promise<void>;
  onUpdate: (id: string, body: PersonaBody) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onClose: () => void;
}) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [form, setForm] = useState<PersonaBody>(EMPTY);
  const [showErrors, setShowErrors] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  // Esc closes; Tab stays inside. A dialog that leaks focus to the chat behind
  // it is worse than no dialog at all for anyone not using a mouse.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== "Tab") return;
      const items = focusablesOf(dialogRef.current);
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !dialogRef.current?.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const errors = validate(form);
  const nameChars = [...form.name.trim()].length;
  const promptChars = [...form.system_prompt.trim()].length;

  const edit = (persona: Persona) => {
    setEditingId(persona.id);
    setForm({ name: persona.name, icon: persona.icon, system_prompt: persona.system_prompt });
    setShowErrors(false);
    setServerError(null);
    nameRef.current?.focus();
  };

  const startNew = () => {
    setEditingId(null);
    setForm(EMPTY);
    setShowErrors(false);
    setServerError(null);
    nameRef.current?.focus();
  };

  const save = async () => {
    setShowErrors(true);
    setServerError(null);
    if (Object.keys(errors).length > 0 || busy) return;
    setBusy(true);
    try {
      if (editingId) await onUpdate(editingId, form);
      else await onCreate(form);
      setForm(EMPTY);
      setEditingId(null);
      setShowErrors(false);
    } catch (err) {
      setServerError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: string) => {
    setServerError(null);
    setBusy(true);
    try {
      await onDelete(id);
      setConfirmId(null);
      if (editingId === id) startNew();
    } catch (err) {
      setServerError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="cmdk__backdrop" onClick={onClose}>
      <div
        ref={dialogRef}
        className="persona-manager"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="persona-manager__head">
          <h2 id={titleId}>Personas</h2>
          <button type="button" className="persona-manager__close" onClick={onClose}>
            Close
          </button>
        </div>

        <div className="persona-manager__body">
          <div className="persona-manager__list">
            {personas.length === 0 && (
              <p className="muted persona-manager__empty">
                No personas yet. Start from a template on the right.
              </p>
            )}
            {personas.map((p) => (
              <div key={p.id} className="persona-row" data-selected={editingId === p.id ? "yes" : "no"}>
                <span className="persona-row__icon" aria-hidden="true">
                  {p.icon}
                </span>
                <div className="persona-row__text">
                  <span className="persona-row__name">{p.name}</span>
                  <span className="persona-row__prompt">{p.system_prompt}</span>
                </div>
                {confirmId === p.id ? (
                  <div className="persona-row__actions">
                    <span className="persona-row__confirm">
                      Delete it? Chats using it fall back to the global default.
                    </span>
                    <button
                      type="button"
                      className="persona-row__danger"
                      disabled={busy}
                      onClick={() => remove(p.id)}
                    >
                      Delete
                    </button>
                    <button type="button" onClick={() => setConfirmId(null)}>
                      Cancel
                    </button>
                  </div>
                ) : (
                  <div className="persona-row__actions">
                    <button type="button" onClick={() => edit(p)}>
                      Edit
                    </button>
                    <button
                      type="button"
                      aria-label={`Delete ${p.name}`}
                      onClick={() => setConfirmId(p.id)}
                    >
                      Delete
                    </button>
                  </div>
                )}
              </div>
            ))}
          </div>

          <form
            className="persona-form"
            onSubmit={(e) => {
              e.preventDefault();
              save();
            }}
          >
            <h3 className="persona-form__title">{editingId ? "Edit persona" : "New persona"}</h3>

            {!editingId && (
              <div className="persona-form__templates">
                {PERSONA_TEMPLATES.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    className="persona-template"
                    onClick={() => {
                      setForm({ name: t.name, icon: t.icon, system_prompt: t.system_prompt });
                      setServerError(null);
                    }}
                  >
                    <span className="persona-template__icon" aria-hidden="true">
                      {t.icon}
                    </span>
                    <span className="persona-template__name">{t.name}</span>
                    <span className="persona-template__blurb">{t.blurb}</span>
                  </button>
                ))}
              </div>
            )}

            <div className="persona-form__row">
              <label className="persona-field persona-field--icon">
                <span className="persona-field__label">Icon</span>
                <input
                  value={form.icon}
                  onChange={(e) => setForm((f) => ({ ...f, icon: e.target.value }))}
                  placeholder="🎯"
                  aria-invalid={showErrors && !!errors.icon}
                />
              </label>
              <label className="persona-field persona-field--name">
                <span className="persona-field__label">
                  Name
                  <span className="persona-field__count numeric" data-over={nameChars > NAME_MAX_CHARS}>
                    {nameChars}/{NAME_MAX_CHARS}
                  </span>
                </span>
                <input
                  ref={nameRef}
                  value={form.name}
                  onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
                  placeholder="Coding partner"
                  aria-invalid={showErrors && !!errors.name}
                />
              </label>
            </div>
            {showErrors && (errors.icon || errors.name) && (
              <p className="persona-form__err">{errors.icon ?? errors.name}</p>
            )}

            <label className="persona-field">
              <span className="persona-field__label">
                System prompt
                <span
                  className="persona-field__count numeric"
                  data-over={promptChars > PROMPT_MAX_CHARS}
                >
                  {promptChars}/{PROMPT_MAX_CHARS}
                </span>
              </span>
              <textarea
                value={form.system_prompt}
                onChange={(e) => setForm((f) => ({ ...f, system_prompt: e.target.value }))}
                rows={7}
                placeholder="How this persona should answer…"
                aria-invalid={showErrors && !!errors.system_prompt}
              />
            </label>
            {showErrors && errors.system_prompt && (
              <p className="persona-form__err">{errors.system_prompt}</p>
            )}

            <p className="persona-form__hint muted">
              The prompt goes to the model exactly as written, ahead of your message.
            </p>

            {serverError && (
              <p className="persona-form__err" role="alert">
                {serverError}
              </p>
            )}

            <div className="persona-form__actions">
              {editingId && (
                <button type="button" onClick={startNew}>
                  New persona
                </button>
              )}
              <button type="submit" className="persona-form__save" disabled={busy}>
                {busy ? "Saving…" : editingId ? "Save changes" : "Create persona"}
              </button>
            </div>
          </form>
        </div>
      </div>
    </div>
  );
}

/** The same rules as the core, phrased for someone looking at the field. */
function validate(form: PersonaBody): FieldErrors {
  const errors: FieldErrors = {};
  const name = form.name.trim();
  const icon = form.icon.trim();
  const prompt = form.system_prompt.trim();

  if (!name) errors.name = "A name is required.";
  else if ([...name].length > NAME_MAX_CHARS)
    errors.name = `The name can be at most ${NAME_MAX_CHARS} characters.`;

  if (!icon) errors.icon = "An icon is required — one emoji.";
  else if (new TextEncoder().encode(icon).length > ICON_MAX_BYTES)
    errors.icon = "The icon has to be a single emoji.";

  if (!prompt) errors.system_prompt = "A system prompt is required.";
  else if ([...prompt].length > PROMPT_MAX_CHARS)
    errors.system_prompt = `The system prompt can be at most ${PROMPT_MAX_CHARS} characters.`;

  return errors;
}

function focusablesOf(dialog: HTMLDivElement | null): HTMLElement[] {
  if (!dialog) return [];
  const selector = "button, input, textarea, select, a[href], [tabindex]:not([tabindex='-1'])";
  return [...dialog.querySelectorAll<HTMLElement>(selector)].filter(
    (el) => !el.hasAttribute("disabled") && el.getAttribute("aria-hidden") !== "true",
  );
}
