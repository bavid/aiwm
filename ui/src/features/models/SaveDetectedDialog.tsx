import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { humanize } from "../../lib/errors";
import { saveBaseFamilies, type SavedFamily } from "../../lib/ipc";
import { SOURCE_PHRASE, type DetectedRow } from "./packages-view";

type Props = {
  rows: DetectedRow[];
  onClose: () => void;
  /** Called after a save, so the grouping is read again. */
  onSaved: () => void;
};

type Phase = { kind: "preview" } | { kind: "busy" } | { kind: "done"; results: SavedFamily[] };

/** Preview of the families the Packages view inferred but the library has
 *  not recorded, and the one button that records them. Weak guesses (from
 *  the file name) start unticked. Same modal rules as `PackageDialog`. */
export function SaveDetectedDialog({ rows: initialRows, onClose, onSaved }: Props) {
  // The preview as opened: the list is re-read after the save, and the result
  // must still name what was in it.
  const [rows] = useState(initialRows);
  const ref = useRef<HTMLDialogElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const [picked, setPicked] = useState<ReadonlySet<string>>(
    () => new Set(rows.filter((r) => !r.weak).map((r) => r.modelId)),
  );
  const [phase, setPhase] = useState<Phase>({ kind: "preview" });
  const [error, setError] = useState<string | null>(null);

  useLayoutEffect(() => {
    const dialog = ref.current;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (dialog && !dialog.open) dialog.showModal();
    cancelRef.current?.focus();
    return () => {
      if (dialog?.open) dialog.close();
      opener?.focus();
    };
  }, []);

  useEffect(() => {
    if (phase.kind === "done") cancelRef.current?.focus();
  }, [phase.kind]);

  const busy = phase.kind === "busy";
  const close = () => {
    if (!busy) onClose();
  };
  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const save = async () => {
    const batch = rows
      .filter((r) => picked.has(r.modelId))
      .map((r) => ({ model_id: r.modelId, family: r.family }));
    if (batch.length === 0) return;
    setPhase({ kind: "busy" });
    setError(null);
    try {
      const results = await saveBaseFamilies(batch);
      setPhase({ kind: "done", results });
      onSaved();
    } catch (e) {
      setError(humanize(e));
      setPhase({ kind: "preview" });
    }
  };

  return (
    <dialog
      ref={ref}
      className="pkg"
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        close();
      }}
    >
      <header className="pkg__head">
        <p className="pkg__eyebrow">
          Record what was detected <HelpHint area="models" setting="save-detected" />
        </p>
        <h3 id={titleId} className="pkg__title">
          Save detected families
        </h3>
        <p className="pkg__licence">
          These families were worked out on read and are not stored yet. Saving writes them to
          the library; a base you chose yourself is never replaced.
        </p>
      </header>
      <div className="pkg__body">
        {phase.kind === "done" ? (
          <SaveResult results={phase.results} rows={rows} />
        ) : (
          <ul className="pkgv-detected" aria-label="Detected families">
            {rows.map((r) => (
              <DetectedLine
                key={r.modelId}
                row={r}
                checked={picked.has(r.modelId)}
                disabled={busy}
                onToggle={() => toggle(r.modelId)}
              />
            ))}
          </ul>
        )}
        {error && (
          <p className="pkg__error" role="alert">
            {error}
          </p>
        )}
      </div>
      <footer className="pkg__foot">
        {phase.kind !== "done" && (
          <span className="pkg__total numeric">
            {picked.size} of {rows.length} selected
          </span>
        )}
        <div className="pkg__actions">
          <button ref={cancelRef} type="button" className="chip" disabled={busy} onClick={close}>
            {phase.kind === "done" ? "Close" : "Cancel"}
          </button>
          {phase.kind !== "done" && (
            <button
              type="button"
              className="pkg__go"
              disabled={busy || picked.size === 0}
              onClick={save}
            >
              {busy ? "Saving…" : `Save ${picked.size} famil${picked.size === 1 ? "y" : "ies"}`}
            </button>
          )}
        </div>
      </footer>
    </dialog>
  );
}

function DetectedLine({
  row,
  checked,
  disabled,
  onToggle,
}: {
  row: DetectedRow;
  checked: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  const id = useId();
  return (
    <li className="pkgv-detected__row" data-weak={row.weak || undefined}>
      <input id={id} type="checkbox" checked={checked} disabled={disabled} onChange={onToggle} />
      <label htmlFor={id}>
        <span className="pkgv-detected__name">{row.name}</span>
        <span className="pkgv-detected__arrow" aria-hidden="true">
          →
        </span>
        <span className="pkgv-detected__family">{row.familyLabel}</span>
        <span className="pkgv-detected__source">
          {SOURCE_PHRASE[row.source]}
          {row.weak && <strong className="pkgv-weak"> · weak guess — check before saving</strong>}
        </span>
      </label>
    </li>
  );
}

function SaveResult({ results, rows }: { results: SavedFamily[]; rows: DetectedRow[] }) {
  const count = (o: SavedFamily["outcome"]) => results.filter((r) => r.outcome === o).length;
  const skipped = results.filter((r) => r.outcome === "skipped");
  const nameOf = (id: string) => rows.find((r) => r.modelId === id)?.name ?? id;
  return (
    <div className="pkgv-result" role="status">
      <p className="pkg__verdict" data-verdict="ready">
        <span aria-hidden="true">✓ </span>
        <span className="numeric">{count("written")}</span> written ·{" "}
        <span className="numeric">{count("kept_user_choice")}</span> kept (your choice) ·{" "}
        <span className="numeric">{skipped.length}</span> skipped (detection changed)
      </p>
      {skipped.length > 0 && (
        <ul className="pkgv-result__skipped">
          {skipped.map((s) => (
            <li key={s.model_id}>
              {nameOf(s.model_id)}: {s.reason ?? "skipped"}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
