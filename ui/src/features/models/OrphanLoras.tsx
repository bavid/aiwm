import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { BASE_FAMILIES } from "../../lib/base-families";
import { humanize } from "../../lib/errors";
import { setModelBaseFamily, type Model } from "../../lib/ipc";
import { formatGB } from "../../lib/units";

type Props = {
  orphans: Model[];
  /** Called after a family was saved, so the grouping is read again. */
  onChanged: () => void;
};

/** LoRAs whose base could not be inferred (no recorded family, no telling
 *  header, nothing in the name): each asks "What base is this for?". */
export function OrphanLoras({ orphans, onChanged }: Props) {
  const titleId = useId();
  if (orphans.length === 0) return null;
  return (
    <section className="pkgv-orphans" aria-labelledby={titleId}>
      <h3 id={titleId} className="pkgv-orphans__title">
        <span aria-hidden="true">? </span>
        LoRAs of unknown base <span className="numeric">({orphans.length})</span>{" "}
        <HelpHint area="models" setting="what-base" />
      </h3>
      <p className="muted">
        Nothing in these files says which base they were trained for. Pick it (the LoRA's page
        on Civitai or Hugging Face says) — the LoRA pickers and this view then place it.
      </p>
      <ul className="pkgv-orphans__list">
        {orphans.map((m) => (
          <OrphanRow key={m.id} model={m} onChanged={onChanged} />
        ))}
      </ul>
    </section>
  );
}

function OrphanRow({ model, onChanged }: { model: Model; onChanged: () => void }) {
  const selectId = useId();
  const [family, setFamily] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    if (!family) return;
    setBusy(true);
    setError(null);
    try {
      await setModelBaseFamily(model.id, family);
      onChanged();
    } catch (e) {
      setError(humanize(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <li className="pkgv-orphan">
      <span className="pkgv-orphan__name">
        {model.name}{" "}
        <span className="muted numeric">{formatGB(model.size_bytes, 2)}</span>
      </span>
      <span className="pkgv-orphan__pick">
        <label htmlFor={selectId}>What base is this for?</label>
        <select
          id={selectId}
          value={family}
          disabled={busy}
          onChange={(e) => setFamily(e.target.value)}
        >
          <option value="">Choose…</option>
          {BASE_FAMILIES.map((f) => (
            <option key={f.id} value={f.id}>
              {f.label}
              {f.notRunnable ? " (not runnable here)" : ""}
            </option>
          ))}
        </select>
        <button type="button" className="pkg__go" disabled={!family || busy} onClick={save}>
          {busy ? "Saving…" : "Save"}
        </button>
      </span>
      {error && (
        <p className="pkg__error" role="alert">
          {error}
        </p>
      )}
    </li>
  );
}
