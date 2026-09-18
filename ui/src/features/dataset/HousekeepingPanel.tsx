import { useId, useState } from "react";
import {
  cleanupDataset,
  DEFAULT_DEDUP_THRESHOLD,
  dedupDataset,
  deleteDataset,
  MAX_DEDUP_THRESHOLD,
  type CleanupSummary,
  type Dataset,
  type DatasetDeleteSummary,
  type DatasetUsage,
  type DedupSummary,
  type SkippedFile,
} from "../../lib/ipc";
import { ConfirmDialog } from "./ConfirmDialog";
import { errorText, framesLabel } from "./curation";
import { formatBytes } from "./format";
import { SkippedFiles } from "./SkippedFiles";
import "./housekeeping.css";

type Props = {
  dataset: Dataset;
  usage: DatasetUsage | null;
  /** A prep run is still writing frames: nothing is deleted under it. */
  isRunning: boolean;
  /** Refetch frames and usage after a dedup or cleanup. */
  onChanged: () => void;
  /** The dataset is gone; the parent leaves it and shows the summary. */
  onDeleted: (summary: DatasetDeleteSummary) => void;
};

type Result = { kind: "ok" | "error"; text: string; skipped?: SkippedFile[] };

type Dialog =
  | { kind: "cleanup"; preview: CleanupSummary }
  | { kind: "delete" };

function dedupText(s: DedupSummary): string {
  if (s.marked === 0) {
    return `No near-duplicates among ${framesLabel(s.scanned)} at threshold ${s.threshold}.`;
  }
  const unreadable = s.unreadable > 0 ? ` ${s.unreadable} could not be read.` : "";
  return (
    `Checked ${framesLabel(s.scanned)} at threshold ${s.threshold}: ` +
    `${s.groups.toLocaleString()} group(s) of near-duplicates, ` +
    `${framesLabel(s.marked)} marked "Duplicate (dataset)" and moved to Discard — the sharpest ` +
    `frame of each group stays in Keep.${unreadable}`
  );
}

/** Dataset-wide housekeeping: disk use, duplicate search, cleanup of the
 *  discarded frames, and deleting the whole dataset. Every deletion is
 *  previewed and confirmed; the core's refusals are shown verbatim. */
export function HousekeepingPanel({ dataset, usage, isRunning, onChanged, onDeleted }: Props) {
  const [threshold, setThreshold] = useState(DEFAULT_DEDUP_THRESHOLD);
  const [busy, setBusy] = useState<"dedup" | "preview" | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [isDialogBusy, setIsDialogBusy] = useState(false);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const thresholdId = useId();
  const thresholdHelpId = useId();
  const isClips = dataset.mode === "clips";

  const openDialog = (next: Dialog) => {
    setDialogError(null);
    setDialog(next);
  };

  const runDedup = async () => {
    setBusy("dedup");
    setResult(null);
    try {
      setResult({ kind: "ok", text: dedupText(await dedupDataset(dataset.id, threshold)) });
      onChanged();
    } catch (e) {
      setResult({ kind: "error", text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const previewCleanup = async () => {
    setBusy("preview");
    setResult(null);
    try {
      const preview = await cleanupDataset(dataset.id, true);
      if (preview.frames === 0) setResult({ kind: "ok", text: "Nothing to clean up." });
      else openDialog({ kind: "cleanup", preview });
    } catch (e) {
      setResult({ kind: "error", text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  const confirm = async () => {
    if (!dialog) return;
    setIsDialogBusy(true);
    setDialogError(null);
    try {
      if (dialog.kind === "cleanup") {
        const s = await cleanupDataset(dataset.id, false);
        setDialog(null);
        setResult({
          kind: "ok",
          text: `Deleted ${framesLabel(s.frames)} (${s.deleted_files.toLocaleString()} files) — ${formatBytes(s.bytes)} freed.`,
          skipped: s.skipped_files,
        });
        onChanged();
      } else {
        const summary = await deleteDataset(dataset.id);
        setDialog(null);
        onDeleted(summary);
      }
    } catch (e) {
      setDialogError(errorText(e));
    } finally {
      setIsDialogBusy(false);
    }
  };

  const exportLine = !usage?.export_dir
    ? "Not exported yet"
    : usage.export_app_owned
      ? `${formatBytes(usage.export_bytes)} · inside the app's folder`
      : "Your own folder — never deleted by the app";

  return (
    <div className="card housekeeping">
      <h3 className="housekeeping__title">Housekeeping</h3>

      <dl className="housekeeping__usage" aria-busy={usage === null}>
        <div>
          <dt>Work folder</dt>
          <dd title={usage?.work_dir ?? undefined}>
            {usage
              ? `${formatBytes(usage.work_bytes)} · ${usage.work_files.toLocaleString()} files`
              : "Measuring…"}
          </dd>
        </div>
        <div>
          <dt>Export</dt>
          <dd>
            {usage ? exportLine : "…"}
            {usage?.export_dir && <span className="housekeeping__path">{usage.export_dir}</span>}
          </dd>
        </div>
        <div>
          <dt>Discarded</dt>
          <dd>
            {usage
              ? `${framesLabel(usage.discarded_frames)} · ${formatBytes(usage.discarded_bytes)}`
              : "…"}
          </dd>
        </div>
      </dl>

      <div className="housekeeping__tools">
        <div className="housekeeping__tool">
          <label className="housekeeping__label" htmlFor={thresholdId}>
            Duplicate threshold <output htmlFor={thresholdId}>{threshold}</output>
          </label>
          <input
            id={thresholdId}
            type="range"
            min={0}
            max={MAX_DEDUP_THRESHOLD}
            step={1}
            value={threshold}
            disabled={isClips}
            aria-describedby={thresholdHelpId}
            onChange={(e) => setThreshold(Number(e.target.value))}
          />
          <p id={thresholdHelpId} className="housekeeping__help">
            {isClips
              ? "Duplicate search works on still frames; this dataset holds clips."
              : `Lower = only near-identical frames; higher also catches similar shots. Default ${DEFAULT_DEDUP_THRESHOLD}. Marked frames move to Discard and can be moved back.`}
          </p>
          <button
            type="button"
            className="chip"
            disabled={isClips || isRunning || busy !== null}
            onClick={() => void runDedup()}
          >
            {busy === "dedup" ? "Searching…" : "Find duplicates"}
          </button>
        </div>

        <div className="housekeeping__tool">
          <span className="housekeeping__label">Clean up</span>
          <p className="housekeeping__help">
            Deletes every frame in Discard with its file, after a preview. Kept frames are never
            touched.
          </p>
          <button
            type="button"
            className="chip"
            disabled={isRunning || busy !== null || usage?.discarded_frames === 0}
            onClick={() => void previewCleanup()}
          >
            {busy === "preview" ? "Measuring…" : "Clean up discarded…"}
          </button>
        </div>

        <div className="housekeeping__tool housekeeping__tool--danger">
          <span className="housekeeping__label">Delete dataset</span>
          <p className="housekeeping__help">
            Removes the dataset, its frames and its work folder. Source videos are never touched.
          </p>
          <button
            type="button"
            className="chip housekeeping__danger"
            disabled={isRunning || busy !== null}
            onClick={() => openDialog({ kind: "delete" })}
          >
            Delete dataset…
          </button>
        </div>
      </div>

      {isRunning && (
        <p className="housekeeping__help">Wait for the prep run to finish before deleting.</p>
      )}
      {result && (
        <div className="housekeeping__result" role={result.kind === "error" ? "alert" : "status"}>
          <p className={result.kind === "error" ? "dataset__err" : "dataset__done"}>{result.text}</p>
          {result.skipped && <SkippedFiles files={result.skipped} />}
        </div>
      )}

      <ConfirmDialog
        isOpen={dialog?.kind === "cleanup"}
        title="Clean up discarded frames?"
        confirmLabel="Delete discarded frames"
        tone="danger"
        isBusy={isDialogBusy}
        error={dialogError}
        onConfirm={() => void confirm()}
        onCancel={() => setDialog(null)}
      >
        {dialog?.kind === "cleanup" && (
          <p>
            <strong>
              {framesLabel(dialog.preview.frames)}, {formatBytes(dialog.preview.bytes)}
            </strong>{" "}
            will be deleted. Their files are removed from disk; your source videos are not touched,
            and kept frames stay as they are.
          </p>
        )}
      </ConfirmDialog>

      <ConfirmDialog
        isOpen={dialog?.kind === "delete"}
        title={`Delete "${dataset.name}"?`}
        confirmLabel="Delete dataset"
        tone="danger"
        isBusy={isDialogBusy}
        error={dialogError}
        onConfirm={() => void confirm()}
        onCancel={() => setDialog(null)}
      >
        <p>This cannot be undone. Removed:</p>
        <ul>
          <li>the dataset with its frames, captions and concepts</li>
          <li>
            the work folder
            {usage
              ? ` — ${formatBytes(usage.work_bytes)} in ${usage.work_files.toLocaleString()} files`
              : ""}
            {usage?.work_dir && <span className="confirm__path">{usage.work_dir}</span>}
          </li>
          {usage?.export_dir && usage.export_app_owned && (
            <li>
              the export inside the app's folder — {formatBytes(usage.export_bytes)}
              <span className="confirm__path">{usage.export_dir}</span>
            </li>
          )}
        </ul>
        <p>Kept:</p>
        <ul>
          <li>your source videos and images ({dataset.source_root})</li>
          {usage?.export_dir && !usage.export_app_owned && (
            <li>
              your export folder, which may hold other files
              <span className="confirm__path">{usage.export_dir}</span>
            </li>
          )}
          <li>finished LoRAs trained from it</li>
        </ul>
      </ConfirmDialog>
    </div>
  );
}
