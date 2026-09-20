import { useId, useLayoutEffect, useRef, useState } from "react";
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
import { openFolder } from "../../lib/browse";
import { HelpHint } from "../../components/HelpHint";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { errorText, filesLabel, framesLabel } from "./curation";
import { formatBytes } from "./format";
import { SkippedFiles } from "../../components/SkippedFiles";
import "./housekeeping.css";

type Props = {
  dataset: Dataset;
  /** Fetched on demand by the parent; `null` while it loads. */
  usage: DatasetUsage | null;
  isUsageLoading: boolean;
  /** Why measuring failed; shown instead of an endless "calculating…". */
  usageError: string | null;
  /** Discarded frames right now, from the live frame list — the usage is
   *  only measured on demand, so its own count can lag behind a move. */
  discardedCount: number;
  /** Measure the disk use again (the core walks the folders). */
  onRefreshUsage: () => void;
  /** A prep run is still writing frames: nothing is deleted under it. */
  isRunning: boolean;
  /** Refetch frames and usage after a dedup, cleanup or failed delete. */
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

const CALCULATING = "calculating…";

/** What deleting the dataset does to its work folder, per the core's
 *  `work_walkable`: the whole folder, or only this dataset's own frame
 *  files when other data points into the folder. */
function workFolderLine(usage: DatasetUsage | null) {
  if (!usage) return `its files — size ${CALCULATING}`;
  const size = `${formatBytes(usage.work_bytes)}, ${filesLabel(usage.work_files)}`;
  if (usage.work_walkable && usage.work_dir) {
    return (
      <>
        the work folder ({size})
        <span className="confirm__path">{usage.work_dir}</span>
      </>
    );
  }
  return (
    <>
      this dataset's own frame files ({size})
      {usage.work_dir && (
        <>
          ; the folder is kept because other data uses it
          <span className="confirm__path">{usage.work_dir}</span>
        </>
      )}
    </>
  );
}

/** Dataset-wide housekeeping: disk use, duplicate search, cleanup of the
 *  discarded frames, and deleting the whole dataset. Every deletion is
 *  previewed and confirmed; the core's refusals are shown verbatim. */
export function HousekeepingPanel({
  dataset,
  usage,
  isUsageLoading,
  usageError,
  discardedCount,
  onRefreshUsage,
  isRunning,
  onChanged,
  onDeleted,
}: Props) {
  const [threshold, setThreshold] = useState(DEFAULT_DEDUP_THRESHOLD);
  const [busy, setBusy] = useState<"dedup" | "preview" | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [isDialogBusy, setIsDialogBusy] = useState(false);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const resultRef = useRef<HTMLDivElement>(null);
  /** Set when a dialog closes with a result: focus goes to it, not to a
   *  button that may just have become disabled. */
  const focusResult = useRef(false);
  const thresholdId = useId();
  const thresholdHelpId = useId();
  const dedupBtnId = useId();
  const cleanupBtnId = useId();
  const deleteBtnId = useId();
  const isClips = dataset.mode === "clips";
  /** Where this dataset's frames live: recorded on the row since Plan 10,
   *  else as measured (older datasets). */
  const workDir = dataset.work_dir ?? usage?.work_dir ?? null;
  /** Why "Open folder" failed — kept apart from the tools' result line. */
  const [revealError, setRevealError] = useState<string | null>(null);

  const revealWorkDir = async () => {
    if (!workDir) return;
    setRevealError(await openFolder(workDir));
  };

  useLayoutEffect(() => {
    if (!focusResult.current || dialog) return;
    focusResult.current = false;
    resultRef.current?.focus();
  });

  const openDialog = (next: Dialog) => {
    setDialogError(null);
    setDialog(next);
    onRefreshUsage();
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
        focusResult.current = true;
        setDialog(null);
        setResult({
          kind: "ok",
          text: `Deleted ${framesLabel(s.frames)} (${filesLabel(s.deleted_files)}) — ${formatBytes(s.bytes)} freed.`,
          skipped: s.skipped_files,
        });
        onChanged();
      } else {
        const summary = await deleteDataset(dataset.id);
        setDialog(null);
        if (summary.dataset_deleted) {
          onDeleted(summary);
        } else {
          // A file could not be deleted: the core kept the dataset so the
          // delete can be retried once the file is free.
          focusResult.current = true;
          setResult({
            kind: "error",
            text:
              `Some files could not be deleted; the dataset was kept so you can retry. ` +
              `${formatBytes(summary.freed_bytes)} freed so far.`,
            skipped: summary.skipped_files,
          });
          onChanged();
        }
      }
    } catch (e) {
      setDialogError(errorText(e));
    } finally {
      setIsDialogBusy(false);
    }
  };

  // Until a measurement lands: why it failed, or that it is under way.
  const pending = usageError ? `could not measure: ${usageError}` : CALCULATING;

  const exportLine = !usage?.export_dir
    ? "Not exported yet"
    : usage.export_app_owned
      ? `${formatBytes(usage.export_bytes)} · inside the app's folder`
      : "Your own folder — never deleted by the app";

  return (
    <div className="card housekeeping">
      <h3 className="housekeeping__title">Housekeeping</h3>

      <div className="housekeeping__usage-head">
        <span className="housekeeping__label">Disk use</span>
        <button
          type="button"
          className="chip housekeeping__refresh"
          disabled={isUsageLoading}
          onClick={onRefreshUsage}
        >
          {isUsageLoading ? "Calculating…" : "Measure again"}
        </button>
      </div>
      <dl className="housekeeping__usage" aria-busy={isUsageLoading}>
        <div>
          <dt>{usage && !usage.work_walkable ? "Own frame files" : "Work folder"}</dt>
          <dd title={usage?.work_dir ?? undefined}>
            {usage
              ? `${formatBytes(usage.work_bytes)} · ${filesLabel(usage.work_files)}`
              : pending}
          </dd>
        </div>
        <div>
          <dt>Export</dt>
          <dd>
            {usage ? exportLine : pending}
            {usage?.export_dir && <span className="housekeeping__path">{usage.export_dir}</span>}
          </dd>
        </div>
        <div>
          <dt>Discarded</dt>
          <dd>
            {framesLabel(discardedCount)}
            <span className="housekeeping__path">
              {usage
                ? `${formatBytes(usage.discarded_bytes)} for ${framesLabel(usage.discarded_frames)} as measured`
                : pending}
            </span>
          </dd>
        </div>
      </dl>

      {workDir && (
        <div className="housekeeping__location">
          <span className="housekeeping__label">Stored in</span>
          <code className="housekeeping__location-path">{workDir}</code>
          <button
            type="button"
            className="chip housekeeping__refresh"
            onClick={() => void revealWorkDir()}
          >
            Open folder
          </button>
          {revealError && (
            <p className="dataset__err housekeeping__location-err" role="alert">
              {revealError}
            </p>
          )}
        </div>
      )}

      <div className="housekeeping__tools">
        <div className="housekeeping__tool">
          <span className="housekeeping__label housekeeping__label--row">
            <label htmlFor={thresholdId}>
              Duplicate threshold <output htmlFor={thresholdId}>{threshold}</output>
            </label>
            <HelpHint area="dataset" setting="dedup-threshold" describes={thresholdId} />
          </span>
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
          <span className="housekeeping__label--row">
            <button
              id={dedupBtnId}
              type="button"
              className="chip"
              disabled={isClips || isRunning || busy !== null}
              onClick={() => void runDedup()}
            >
              {busy === "dedup" ? "Searching…" : "Find duplicates"}
            </button>
            <HelpHint area="dataset" setting="find-duplicates" describes={dedupBtnId} />
          </span>
        </div>

        <div className="housekeeping__tool">
          <span className="housekeeping__label housekeeping__label--row">
            Clean up
            <HelpHint area="dataset" setting="clean-up" describes={cleanupBtnId} />
          </span>
          <p className="housekeeping__help">
            Deletes every frame in Discard with its file, after a preview. Kept frames are never
            touched.
          </p>
          <button
            id={cleanupBtnId}
            type="button"
            className="chip"
            disabled={isRunning || busy !== null || discardedCount === 0}
            onClick={() => void previewCleanup()}
          >
            {busy === "preview" ? "Measuring…" : "Clean up discarded…"}
          </button>
        </div>

        <div className="housekeeping__tool housekeeping__tool--danger">
          <span className="housekeeping__label housekeeping__label--row">
            Delete dataset
            <HelpHint area="dataset" setting="delete-dataset" describes={deleteBtnId} />
          </span>
          <p className="housekeeping__help">
            Removes the dataset, its frames and its work folder. Source videos are never touched.
          </p>
          <button
            id={deleteBtnId}
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
      {/* Always mounted, so screen readers pick up the text when it arrives. */}
      <div
        ref={resultRef}
        className="housekeeping__result"
        role="status"
        aria-live="polite"
        tabIndex={-1}
      >
        {result && (
          <>
            <p className={result.kind === "error" ? "dataset__err" : "dataset__done"}>
              {result.text}
            </p>
            {result.skipped && <SkippedFiles files={result.skipped} />}
          </>
        )}
      </div>

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
          <li>{workFolderLine(usage)}</li>
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
