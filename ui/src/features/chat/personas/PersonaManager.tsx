import { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Persona, PersonaBody } from "../../../lib/ipc";
import { PERSONA_TEMPLATES } from "./persona-templates";

/** The core's own limits (`core::persona::validate`), mirrored here so the
 *  form can say what is wrong before a round-trip. The server still decides:
 *  whatever it rejects is shown verbatim next to the field. */
const NAME_MAX_CHARS = 60;
/** Bytes, not characters: one emoji can be a long ZWJ sequence. */
const ICON_MAX_BYTES = 64;
const PROMPT_MAX_CHARS = 8000;
/** Unicode's control category — what Rust's `char::is_control` matches. Both
 *  the name and the icon are rendered inline (chip, job event line), so a
 *  newline in either would forge a second line. */
const CONTROL_CHARS = /\p{Cc}/u;

const EMPTY: PersonaBody = { name: "", icon: "", system_prompt: "" };

type FieldErrors = { name?: string; icon?: string; system_prompt?: string };

/** Which row's Delete button to put focus on once a deletion has actually
 *  landed — the list is re-read from the server, so this waits for `id` to be
 *  gone rather than guessing when that happens. */
type FocusAfterDelete = { id: string; index: number };

/** Create, edit and delete personas. A modal because it is a detour from the
 *  conversation, not part of it — Esc and the backdrop both get you back,
 *  after asking when there is unsaved text in the form. */
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
  const iconLabelId = useId();
  const iconInputId = useId();
  const iconErrorId = useId();
  const nameLabelId = useId();
  const nameInputId = useId();
  const nameErrorId = useId();
  const promptLabelId = useId();
  const promptInputId = useId();
  const promptErrorId = useId();
  const confirmTextId = useId();

  const dialogRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [form, setForm] = useState<PersonaBody>(EMPTY);
  const [showErrors, setShowErrors] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [focusAfterDelete, setFocusAfterDelete] = useState<FocusAfterDelete | null>(null);
  const [busy, setBusy] = useState(false);

  const editing = editingId ? personas.find((p) => p.id === editingId) : undefined;
  const baseline: PersonaBody = editing
    ? { name: editing.name, icon: editing.icon, system_prompt: editing.system_prompt }
    : EMPTY;
  const dirty =
    form.name !== baseline.name ||
    form.icon !== baseline.icon ||
    form.system_prompt !== baseline.system_prompt;

  const requestClose = useCallback(() => {
    if (dirty && !window.confirm("Discard the unsaved changes to this persona?")) return;
    onClose();
  }, [dirty, onClose]);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  // A confirmation that appears where the button you just pressed used to be
  // has to take the focus with it, or a keyboard user is left pointing at
  // nothing and never hears the warning.
  useEffect(() => {
    if (confirmId) confirmRef.current?.focus();
  }, [confirmId]);

  // Deleting removes the row that held focus. Wait for the list to actually
  // lose it, then land on the row that took its place (or the last one, or the
  // form when the list is now empty).
  useEffect(() => {
    if (!focusAfterDelete) return;
    if (personas.some((p) => p.id === focusAfterDelete.id)) return;
    const edits = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-edit]") ?? [])];
    (edits[focusAfterDelete.index] ?? edits[edits.length - 1] ?? nameRef.current)?.focus();
    setFocusAfterDelete(null);
  }, [personas, focusAfterDelete]);

  // Esc closes; Tab stays inside. A dialog that leaks focus to the chat behind
  // it is worse than no dialog at all for anyone not using a mouse.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        requestClose();
        return;
      }
      if (e.key !== "Tab") return;
      const items = focusablesOf(dialogRef.current);
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      // Focus outside the dialog entirely (the page body after a click on the
      // backdrop, say) is pulled back in first -- checking it against the
      // first/last row would otherwise miss and let Tab walk the page behind.
      if (!dialogRef.current?.contains(active)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
        return;
      }
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [requestClose]);

  const errors = validate(form);
  const nameChars = [...form.name.trim()].length;
  const promptChars = [...form.system_prompt.trim()].length;
  const showIconError = showErrors && !!errors.icon;
  const showNameError = showErrors && !!errors.name;
  const showPromptError = showErrors && !!errors.system_prompt;

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

  const remove = async (id: string, index: number) => {
    setServerError(null);
    setBusy(true);
    try {
      await onDelete(id);
      setConfirmId(null);
      // Deleting the persona being edited moves focus into the form instead,
      // so the two focus moves never fight over it.
      if (editingId === id) startNew();
      else setFocusAfterDelete({ id, index });
    } catch (err) {
      setServerError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return createPortal(
    <div
      className="cmdk__backdrop"
      role="presentation"
      onMouseDown={(e) => {
        // Only a press that starts *on* the backdrop closes: a drag that began
        // inside the dialog (selecting prompt text) must not.
        if (e.target === e.currentTarget) requestClose();
      }}
    >
      <div
        ref={dialogRef}
        className="persona-manager"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="persona-manager__head">
          <h2 id={titleId}>Personas</h2>
          <button type="button" className="persona-manager__close" onClick={requestClose}>
            Close
          </button>
        </div>

        <div className="persona-manager__body">
          <div className="persona-manager__list" ref={listRef}>
            {personas.length === 0 && (
              <p className="muted persona-manager__empty">
                No personas yet. Start from a template on the right.
              </p>
            )}
            {personas.map((p, index) => (
              <div
                key={p.id}
                className="persona-row"
                data-selected={editingId === p.id ? "yes" : "no"}
              >
                <span className="persona-row__icon" aria-hidden="true">
                  {p.icon}
                </span>
                <div className="persona-row__text">
                  <span className="persona-row__name">{p.name}</span>
                  <span className="persona-row__prompt">{p.system_prompt}</span>
                </div>
                {confirmId === p.id ? (
                  <div className="persona-row__actions">
                    <span className="persona-row__confirm" id={confirmTextId} role="alert">
                      Delete “{p.name}”? Chats using it fall back to the global default.
                    </span>
                    <button
                      ref={confirmRef}
                      type="button"
                      className="persona-row__danger"
                      aria-describedby={confirmTextId}
                      disabled={busy}
                      onClick={() => remove(p.id, index)}
                    >
                      Delete
                    </button>
                    <button type="button" onClick={() => setConfirmId(null)}>
                      Cancel
                    </button>
                  </div>
                ) : (
                  <div className="persona-row__actions">
                    <button
                      type="button"
                      data-edit=""
                      aria-label={`Edit ${p.name}`}
                      onClick={() => edit(p)}
                    >
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
              <div className="persona-field persona-field--icon">
                <div className="persona-field__label">
                  <label id={iconLabelId} htmlFor={iconInputId}>
                    Icon
                  </label>
                </div>
                <input
                  id={iconInputId}
                  aria-labelledby={iconLabelId}
                  value={form.icon}
                  onChange={(e) => setForm((f) => ({ ...f, icon: e.target.value }))}
                  placeholder="🎯"
                  aria-invalid={showIconError}
                  aria-describedby={showIconError ? iconErrorId : undefined}
                />
              </div>
              <div className="persona-field persona-field--name">
                <div className="persona-field__label">
                  <label id={nameLabelId} htmlFor={nameInputId}>
                    Name
                  </label>
                  {/* Outside the label on purpose: as part of the accessible
                      name a screen reader would re-read the whole count on
                      every keystroke. */}
                  <Count used={nameChars} max={NAME_MAX_CHARS} />
                </div>
                <input
                  id={nameInputId}
                  ref={nameRef}
                  aria-labelledby={nameLabelId}
                  value={form.name}
                  onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
                  placeholder="Coding partner"
                  aria-invalid={showNameError}
                  aria-describedby={showNameError ? nameErrorId : undefined}
                />
              </div>
            </div>
            {showIconError && (
              <p className="persona-form__err" id={iconErrorId}>
                {errors.icon}
              </p>
            )}
            {showNameError && (
              <p className="persona-form__err" id={nameErrorId}>
                {errors.name}
              </p>
            )}

            <div className="persona-field">
              <div className="persona-field__label">
                <label id={promptLabelId} htmlFor={promptInputId}>
                  System prompt
                </label>
                <Count used={promptChars} max={PROMPT_MAX_CHARS} />
              </div>
              <textarea
                id={promptInputId}
                aria-labelledby={promptLabelId}
                value={form.system_prompt}
                onChange={(e) => setForm((f) => ({ ...f, system_prompt: e.target.value }))}
                rows={7}
                placeholder="How this persona should answer…"
                aria-invalid={showPromptError}
                aria-describedby={showPromptError ? promptErrorId : undefined}
              />
            </div>
            {showPromptError && (
              <p className="persona-form__err" id={promptErrorId}>
                {errors.system_prompt}
              </p>
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
    </div>,
    document.body,
  );
}

/** A live character count. Decorative for assistive tech (the message that
 *  matters is the field error), and marked with a sign as well as a colour
 *  once it is over the limit. */
function Count({ used, max }: { used: number; max: number }) {
  const over = used > max;
  return (
    <span className="persona-field__count numeric" data-over={over} aria-hidden="true">
      {over ? "⚠ " : ""}
      {used}/{max}
    </span>
  );
}

/** The same rules, in the same order, as the core — phrased for someone
 *  looking at the field rather than at an API response. */
function validate(form: PersonaBody): FieldErrors {
  const errors: FieldErrors = {};
  const name = form.name.trim();
  const icon = form.icon.trim();
  const prompt = form.system_prompt.trim();

  if (!name) errors.name = "A name is required.";
  else if ([...name].length > NAME_MAX_CHARS)
    errors.name = `The name can be at most ${NAME_MAX_CHARS} characters.`;
  else if (CONTROL_CHARS.test(name)) errors.name = "The name cannot contain control characters.";

  if (!icon) errors.icon = "An icon is required — one emoji.";
  else if (CONTROL_CHARS.test(icon)) errors.icon = "The icon cannot contain control characters.";
  else if (new TextEncoder().encode(icon).length > ICON_MAX_BYTES)
    errors.icon = `The icon can be at most ${ICON_MAX_BYTES} bytes — one emoji.`;

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
