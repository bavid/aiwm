import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { BASE_FAMILIES } from "../../lib/base-families";
import { humanize } from "../../lib/errors";
import { setModelBaseFamily, type Model, type UnknownModel } from "../../lib/ipc";
import { formatGB } from "../../lib/units";
import { unknownSummary } from "./packages-view";

type Props = {
  unknown: UnknownModel[];
  /** Called after a family was saved, so the grouping is read again. */
  onChanged: () => void;
};

/** Checkpoints and LoRAs whose base could not be inferred — a base this app
 *  has no stack for (Krea 2, Anima, …) or a file that says nothing. The
 *  checkpoints are listed with their size (they take disk space and run
 *  nowhere here); each LoRA asks "What base is this for?". */
export function OrphanLoras({ unknown, onChanged }: Props) {
  const titleId = useId();
  if (unknown.length === 0) return null;
  const sum = unknownSummary(unknown);
  const checkpoints = unknown.filter((u) => u.kind === "checkpoint");
  const loras = unknown.filter((u) => u.kind === "lora");
  return (
    <section className="pkgv-orphans" aria-labelledby={titleId}>
      <h3 id={titleId} className="pkgv-orphans__title">
        <span aria-hidden="true">? </span>
        Unknown or unsupported base <HelpHint area="models" setting="what-base" />
      </h3>
      <p className="pkgv-orphans__sum numeric">
        {[
          sum.checkpoints > 0 &&
            `${sum.checkpoints} checkpoint${sum.checkpoints === 1 ? "" : "s"} (${formatGB(sum.checkpointBytes, 1)})`,
          sum.loras > 0 && `${sum.loras} LoRA${sum.loras === 1 ? "" : "s"}`,
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>
      <p className="muted">
        Nothing in these files says which base they are for, or they are for a base this app
        cannot run (Krea 2, Anima, …). Checkpoints here stay unused. For a LoRA, pick its base (the
        LoRA's page on Civitai or Hugging Face says) — the LoRA pickers and this view then place it.
      </p>
      {checkpoints.length > 0 && (
        <ul className="pkgv-orphans__list" aria-label="Checkpoints of unknown base">
          {checkpoints.map((u) => (
            <li key={u.model.id} className="pkgv-orphan">
              <span className="pkgv-orphan__name">
                <span className="pkgv-orphan__kind">Checkpoint</span> {u.model.name}{" "}
                <span className="muted numeric">{formatGB(u.model.size_bytes, 1)}</span>
              </span>
            </li>
          ))}
        </ul>
      )}
      {loras.length > 0 && (
        <ul className="pkgv-orphans__list" aria-label="LoRAs of unknown base">
          {loras.map((u) => (
            <OrphanRow key={u.model.id} model={u.model} onChanged={onChanged} />
          ))}
        </ul>
      )}
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
        <span className="pkgv-orphan__kind">LoRA</span> {model.name}{" "}
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
