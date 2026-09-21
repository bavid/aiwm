import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { humanize } from "../../lib/errors";
import { useKnownModels } from "../../lib/hooks";
import {
  civitaiModel,
  enqueueDownload,
  resolvePackage,
  type EnqueueDownloadBody,
  type ModelType,
  type Package,
  type RegistryDetails,
  type RemoteModel,
} from "../../lib/ipc";
import { formatGB } from "../../lib/units";
import { FitBadge } from "./FitBadge";
import { tagPackage } from "./package-groups";
import {
  initialChoices,
  itemDownload,
  plannedBytes,
  plannedNeeds,
  primaryBaseLabel,
  primaryFile,
  type NeedChoice,
} from "./package-plan";
import { PackageNeedRow } from "./PackageNeedRow";
import "./package-dialog.css";

type Props = {
  model: RemoteModel;
  /** The card's already-loaded file list, if "Files" was opened. */
  details: RegistryDetails | null;
  modelType: ModelType;
  /** Whether the search shows adult content — the checkpoint suggestions follow it. */
  showNsfw: boolean;
  onClose: () => void;
  onViewDownloads: () => void;
};

type Loaded = { pkg: Package; details: RegistryDetails };
type Phase = { kind: "idle" } | { kind: "busy" } | { kind: "queued"; files: number; bytes: number };

const KIND_WORD: Record<Package["item"]["kind"], string> = {
  lora: "LoRA",
  checkpoint: "checkpoint",
  other: "model",
};

/** "Get" for a Civitai pick: resolves what the model needs against the
 *  library and the catalogue, shows each need as installed / download /
 *  choose / not runnable, and queues the model alone or with what is
 *  missing — every file through the regular download queue, with its
 *  origin so the import records the base family.
 *
 *  A native modal `<dialog>` with the same focus rules as `ConfirmDialog`
 *  (Cancel takes the first focus, Esc closes, focus returns to the opener);
 *  not `ConfirmDialog` itself, because this one has two actions. */
export function PackageDialog({ model, details, modelType, showNsfw, onClose, onViewDownloads }: Props) {
  const ref = useRef<HTMLDialogElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const viewRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const bodyId = useId();
  const known = useKnownModels();
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [choices, setChoices] = useState<NeedChoice[]>([]);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
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
    let alive = true;
    Promise.all([
      resolvePackage({ source: "civitai", model_id: model.id, nsfw: showNsfw }),
      details ? Promise.resolve(details) : civitaiModel(model.id),
    ])
      .then(([pkg, d]) => {
        if (!alive) return;
        setLoaded({ pkg, details: d });
        setChoices(initialChoices(pkg.needs));
      })
      .catch((e) => alive && setLoadError(humanize(e)));
    return () => {
      alive = false;
    };
  }, [model.id, details, showNsfw]);

  useEffect(() => {
    if (phase.kind === "queued") viewRef.current?.focus();
  }, [phase.kind]);

  const busy = phase.kind === "busy";
  const close = () => {
    if (!busy) onClose();
  };

  const pkg = loaded?.pkg ?? null;
  const file = loaded ? primaryFile(loaded.details) : null;
  const kindWord = pkg ? KIND_WORD[pkg.item.kind] : "model";
  const itemInstalled = !!pkg?.item.model_id;
  const notRunnable = pkg?.verdict.kind === "not_runnable";
  // Nothing to complete: the base is unrunnable, or unknown to this app.
  const offersMissing = !!pkg && !notRunnable && pkg.verdict.kind !== "unknown_base";
  const planned = pkg && known ? plannedNeeds(pkg, choices, known) : [];
  const missingBytes = plannedBytes(planned);
  const missingLabel = planned.length === 0 ? "none" : formatGB(missingBytes, 1);
  const itemBody: EnqueueDownloadBody | null =
    loaded && file ? itemDownload(loaded.details, file, modelType, pkg?.item.base_label ?? primaryBaseLabel(loaded.details)) : null;

  const queue = async (withNeeds: boolean) => {
    const bodies = [
      ...(itemBody && !itemInstalled ? [itemBody] : []),
      ...(withNeeds ? planned.map((p) => p.body) : []),
    ];
    if (bodies.length === 0) return;
    setPhase({ kind: "busy" });
    setError(null);
    const ids: string[] = [];
    try {
      // One at a time, in order: the model first, then what it needs.
      for (const body of bodies) ids.push((await enqueueDownload(body)).id);
      tagPackage(pkg?.item.name ?? model.name ?? model.id, ids);
      setPhase({ kind: "queued", files: bodies.length, bytes: bodies.reduce((s, b) => s + (b.size_bytes ?? 0), 0) });
    } catch (e) {
      tagPackage(pkg?.item.name ?? model.name ?? model.id, ids);
      const done = ids.length > 0 ? ` (${ids.length} of ${bodies.length} already queued)` : "";
      setError(`${humanize(e)}${done}`);
      setPhase({ kind: "idle" });
    }
  };

  const setChoice = (i: number, next: NeedChoice) =>
    setChoices((prev) => prev.map((c, j) => (j === i ? next : c)));

  return (
    <dialog
      ref={ref}
      className="pkg"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
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
          Get a {kindWord} with what it needs <HelpHint area="models" setting="get" />
        </p>
        <h3 id={titleId} className="pkg__title">
          {model.name ?? model.id}
        </h3>
        <div className="pkg__facts">
          {model.model_kind_hint && <span className="badge">{model.model_kind_hint}</span>}
          {pkg?.family && <span className="badge">for {pkg.family.label}</span>}
          {pkg && !pkg.family && pkg.item.base_label && (
            <span className="badge">for “{pkg.item.base_label}”</span>
          )}
          {file && <span className="numeric muted">{formatGB(file.size_bytes, 2)}</span>}
          {file && <FitBadge fit={file.fit} vramMb={file.vram_estimate_mb} subject={model.name ?? model.id} />}
          {itemInstalled && <span className="badge badge--pick">✓ in your library</span>}
        </div>
        {model.allow_commercial_use.length > 0 && (
          <p className="pkg__licence">commercial use: {model.allow_commercial_use.join(", ")}</p>
        )}
      </header>

      <div id={bodyId} className="pkg__body">
        {!loaded && !loadError && <p className="muted">Checking what you already have…</p>}
        {loadError && (
          <p className="pkg__error" role="alert">
            {loadError}
          </p>
        )}
        {pkg && <Verdict pkg={pkg} kindWord={kindWord} />}
        {pkg && !file && (
          <p className="pkg__warnline">Civitai lists no downloadable file for this version.</p>
        )}
        {pkg && pkg.needs.length > 0 && (
          <ul className="pkg__needs" aria-label="What it needs">
            {pkg.needs.map((n, i) => (
              <PackageNeedRow
                key={`${n.role}-${n.label}-${n.optional}`}
                need={n}
                choice={choices[i] ?? { include: false, candidate: 0 }}
                known={known}
                disabled={busy || phase.kind === "queued"}
                onChange={(next) => setChoice(i, next)}
              />
            ))}
          </ul>
        )}
        {error && (
          <p className="pkg__error" role="alert">
            {error}
          </p>
        )}
        <p className="pkg__done" role="status">
          {phase.kind === "queued" &&
            (phase.files === 1
              ? `Queued 1 file (${formatGB(phase.bytes, 1)}) — it is downloaded, verified, then imported.`
              : `Queued ${phase.files} files (${formatGB(phase.bytes, 1)}) — they download one at a time, are verified, then imported.`)}
        </p>
      </div>

      <footer className="pkg__foot">
        {offersMissing && (
          <span className="pkg__total numeric">
            {missingBytes > 0 ? `${formatGB(missingBytes, 1)} to download for what is missing` : "Nothing missing"}
          </span>
        )}
        <div className="pkg__actions">
          <button ref={cancelRef} type="button" className="chip" disabled={busy} onClick={close}>
            {phase.kind === "queued" ? "Close" : "Cancel"}
          </button>
          {phase.kind === "queued" ? (
            <button ref={viewRef} type="button" className="pkg__go" onClick={onViewDownloads}>
              View downloads
            </button>
          ) : (
            <>
              {!itemInstalled && (
                <button
                  type="button"
                  className={notRunnable || (offersMissing && planned.length > 0) ? "pkg__alt" : "pkg__go"}
                  disabled={busy || !itemBody}
                  onClick={() => queue(false)}
                >
                  Download {kindWord} only
                </button>
              )}
              {offersMissing && (
                <button
                  type="button"
                  className="pkg__go"
                  disabled={busy || planned.length === 0}
                  onClick={() => queue(true)}
                >
                  {busy
                    ? "Queuing…"
                    : itemInstalled
                      ? `Download missing (${missingLabel})`
                      : `Download ${kindWord} + missing (${missingLabel})`}
                </button>
              )}
            </>
          )}
        </div>
      </footer>
    </dialog>
  );
}

/** The one-line answer before the details: can it run, and what is missing. */
function Verdict({ pkg, kindWord }: { pkg: Package; kindWord: string }) {
  const v = pkg.verdict;
  switch (v.kind) {
    case "ready":
      return (
        <p className="pkg__verdict" data-verdict="ready">
          <span aria-hidden="true">✓ </span>
          Everything this {kindWord} needs is already installed.
        </p>
      );
    case "needs_download":
      return (
        <p className="pkg__verdict" data-verdict="needs_download">
          <span aria-hidden="true">↓ </span>
          Some of what this {kindWord} needs is not installed yet — pick below.
        </p>
      );
    case "not_runnable":
      return (
        <p className="pkg__verdict" data-verdict="not_runnable">
          <span aria-hidden="true">✗ </span>
          Made for {pkg.family?.label ?? pkg.item.base_label ?? "a base this app cannot run"}: {v.reason}.
          This {kindWord} will not load here.
        </p>
      );
    case "unknown_base":
      return (
        <p className="pkg__verdict" data-verdict="unknown_base">
          <span aria-hidden="true">? </span>
          {pkg.item.base_label
            ? `Civitai lists this ${kindWord} for “${pkg.item.base_label}”, which this app doesn't know — it may not load.`
            : `Civitai does not say which base this ${kindWord} is for — it may not load.`}
        </p>
      );
  }
}
