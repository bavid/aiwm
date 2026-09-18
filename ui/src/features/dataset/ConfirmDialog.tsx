import { useEffect, useId, useRef, type ReactNode } from "react";
import "./confirm-dialog.css";

type Props = {
  isOpen: boolean;
  title: string;
  children: ReactNode;
  confirmLabel: string;
  /** `danger` paints the confirm button in the warning colour — for
   *  deletions that cannot be undone. */
  tone?: "danger" | "default";
  isBusy?: boolean;
  /** Shown inside the dialog, e.g. a refusal from the core, verbatim. */
  error?: string | null;
  onConfirm: () => void;
  onCancel: () => void;
};

/** A modal confirmation on the native `<dialog>`: it traps focus, closes on
 *  Esc and hands focus back to the button that opened it. Cancel takes the
 *  initial focus, so a stray Enter never confirms a deletion. */
export function ConfirmDialog({
  isOpen,
  title,
  children,
  confirmLabel,
  tone = "default",
  isBusy = false,
  error = null,
  onConfirm,
  onCancel,
}: Props) {
  const ref = useRef<HTMLDialogElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (isOpen && !dialog.open) {
      dialog.showModal();
      cancelRef.current?.focus();
    } else if (!isOpen && dialog.open) {
      dialog.close();
    }
  }, [isOpen]);

  return (
    <dialog
      ref={ref}
      className="confirm"
      aria-labelledby={titleId}
      onCancel={(e) => {
        // The browser's own Esc: let the parent decide (it owns `isOpen`).
        e.preventDefault();
        if (!isBusy) onCancel();
      }}
      onKeyDown={(e) => {
        // Esc handled here too, so it works wherever the native close request
        // does not fire (it needs a trusted key event); never mid-request.
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        if (!isBusy) onCancel();
      }}
    >
      <h3 id={titleId} className="confirm__title">
        {title}
      </h3>
      <div className="confirm__body">{children}</div>
      {error && (
        <p className="confirm__error" role="alert">
          {error}
        </p>
      )}
      <div className="confirm__actions">
        <button ref={cancelRef} type="button" className="chip" disabled={isBusy} onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="confirm__go"
          data-tone={tone}
          disabled={isBusy}
          onClick={onConfirm}
        >
          {isBusy ? "Working…" : confirmLabel}
        </button>
      </div>
    </dialog>
  );
}
