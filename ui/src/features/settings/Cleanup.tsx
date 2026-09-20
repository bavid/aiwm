import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { SkippedFiles } from "../../components/SkippedFiles";
import {
  cleanupApply,
  cleanupLog,
  cleanupScan,
  type CleanupApplyResult,
  type CleanupLogEntry,
  type CleanupReport,
} from "../../lib/ipc";
import { formatBytes } from "../../lib/units";
import {
  countLabel,
  EMPTY_SELECTION,
  formatStamp,
  freedLabel,
  GROUP_LABEL,
  pruneSelection,
  selectionTotals,
  setGroup,
  toggleEntry,
  toSelections,
  type Selection,
} from "./cleanup-groups";
import { CleanupGroupCard } from "./CleanupGroupCard";
import { CleanupHistory } from "./CleanupHistory";
import { CleanupPreviewDialog } from "./CleanupPreviewDialog";
import "./cleanup.css";

const HISTORY_LIMIT = 20;

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** What is happening right now, for the busy states and the live region. */
type Phase = "idle" | "scanning" | "previewing" | "deleting";

/** The Settings "Cleanup" section: scan on demand, pick groups or entries,
 *  preview the exact files (a dry run), delete behind the confirmation,
 *  then re-scan. The core's refusals — a busy dataset or run, a download
 *  still going — are shown verbatim and the selection is kept. */
export function Cleanup() {
  const [report, setReport] = useState<CleanupReport | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [phase, setPhase] = useState<Phase>("idle");
  const [preview, setPreview] = useState<CleanupApplyResult | null>(null);
  /** A refusal of the preview (dry run) — shown under the footer. */
  const [previewError, setPreviewError] = useState<string | null>(null);
  /** A refusal of the real run — shown inside the open dialog. */
  const [dialogError, setDialogError] = useState<string | null>(null);
  const [result, setResult] = useState<CleanupApplyResult | null>(null);
  const [history, setHistory] = useState<CleanupLogEntry[]>([]);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const resultRef = useRef<HTMLElement>(null);
  /** Set when a delete finished: focus goes to the result panel once the
   *  dialog has closed, not to a Preview button that just got disabled. */
  const focusResult = useRef(false);

  const isBusy = phase !== "idle";
  const totals = selectionTotals(report, selection);
  const hasSelection = totals.items > 0;
  const groupLabel = useCallback(
    (key: string) => report?.groups.find((g) => g.key === key)?.label ?? GROUP_LABEL[key] ?? key,
    [report],
  );

  const loadHistory = useCallback(async () => {
    try {
      setHistory(await cleanupLog(HISTORY_LIMIT));
      setHistoryError(null);
    } catch (e) {
      setHistoryError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  useLayoutEffect(() => {
    if (!focusResult.current || preview) return;
    focusResult.current = false;
    resultRef.current?.focus();
  });

  const scan = async () => {
    setPhase("scanning");
    setScanError(null);
    setPreviewError(null);
    try {
      const next = await cleanupScan();
      setReport(next);
      setSelection((s) => pruneSelection(s, next));
    } catch (e) {
      setScanError(errorText(e));
    } finally {
      setPhase("idle");
    }
  };

  const runPreview = async () => {
    if (!hasSelection) return;
    setPhase("previewing");
    setPreviewError(null);
    setDialogError(null);
    setResult(null);
    try {
      setPreview(await cleanupApply({ selections: toSelections(selection), dry_run: true }));
    } catch (e) {
      // Refused as a whole (a busy dataset, a run still going): nothing
      // changed, the selection stays so it can be retried later.
      setPreviewError(errorText(e));
    } finally {
      setPhase("idle");
    }
  };

  const confirmDelete = async () => {
    setPhase("deleting");
    setDialogError(null);
    try {
      const done = await cleanupApply({ selections: toSelections(selection), dry_run: false });
      focusResult.current = true;
      setPreview(null);
      setResult(done);
      setSelection(EMPTY_SELECTION);
      setPhase("scanning");
      await Promise.all([
        cleanupScan().then(setReport, (e) => setScanError(errorText(e))),
        loadHistory(),
      ]);
    } catch (e) {
      setDialogError(errorText(e));
    } finally {
      setPhase("idle");
    }
  };

  const nonEmpty = report?.groups.filter((g) => g.entries.length > 0) ?? [];
  const empty = report?.groups.filter((g) => g.entries.length === 0) ?? [];

  const status =
    phase === "scanning"
      ? "Scanning…"
      : phase === "previewing"
        ? "Preparing the preview…"
        : phase === "deleting"
          ? "Deleting…"
          : report
            ? `Scanned at ${formatStamp(report.scanned_at)}`
            : "";

  return (
    <div className="cleanup">
      <section className="card set-group" aria-labelledby="cleanup-title">
        <header className="card__head">
          <h2 id="cleanup-title">Cleanup</h2>
          <span className="card__sub">
            scans on demand <HelpHint area="settings" setting="cleanup-page" />
          </span>
        </header>
        <p className="cleanup__intro">
          This page finds files the app itself created. Models, runtimes, your source media, voice
          identities and anything a running job needs are never listed.
        </p>
        <div className="cleanup__toolbar">
          <button
            type="button"
            className="set-cleanup__go"
            disabled={isBusy}
            aria-busy={phase === "scanning"}
            onClick={() => void scan()}
          >
            {phase === "scanning" ? "Scanning…" : report ? "Scan again" : "Scan"}
          </button>
          <span className="cleanup__status muted" role="status" aria-live="polite">
            {status}
          </span>
        </div>
        {scanError && (
          <p className="settings__err" role="alert">
            Scan failed: {scanError}
          </p>
        )}
      </section>

      {result && (
        <section
          ref={resultRef}
          className="card set-group cleanup-result"
          tabIndex={-1}
          aria-labelledby="cleanup-result-title"
        >
          <h3 id="cleanup-result-title" className="cleanup-result__title">
            Cleanup done
          </h3>
          <p className="numeric">
            Deleted {countLabel(result.deleted_files, "file")} —{" "}
            {freedLabel(result.freed_bytes, result.removed_rows, formatBytes)}{" "}
            {result.removed_rows > 0 ? "removed" : "freed"}.
          </p>
          <SkippedFiles files={result.skipped} />
        </section>
      )}

      {nonEmpty.map((group) => (
        <CleanupGroupCard
          key={group.key}
          group={group}
          selected={selection[group.key] ?? []}
          isDisabled={isBusy}
          onToggleEntry={(id) => setSelection((s) => toggleEntry(s, group.key, id))}
          onToggleGroup={(on) =>
            setSelection((s) =>
              setGroup(
                s,
                group.key,
                group.entries.map((e) => e.id),
                on,
              ),
            )
          }
        />
      ))}

      {report && nonEmpty.length === 0 && (
        <p className="cleanup__empty muted">Nothing to clean up — every group is empty.</p>
      )}
      {empty.length > 0 && nonEmpty.length > 0 && (
        <p className="cleanup__empty muted">
          Nothing to clean up in: {empty.map((g) => g.label).join(", ")}.
        </p>
      )}

      {report && nonEmpty.length > 0 && (
        <div className="cleanup__footer">
          <span className="cleanup__selected numeric">
            Selected: <strong>{freedLabel(totals.bytes, totals.rows, formatBytes)}</strong> in{" "}
            {countLabel(totals.items, "item")}
          </span>
          <button
            type="button"
            className="cleanup__preview"
            disabled={!hasSelection || isBusy}
            aria-busy={phase === "previewing"}
            onClick={() => void runPreview()}
          >
            {phase === "previewing" ? "Preparing…" : "Preview"}
          </button>
          {previewError && (
            <p className="cleanup__refused" role="alert">
              Not started: {previewError.replace(/\.\s*$/, "")}. Your selection is kept.
            </p>
          )}
        </div>
      )}

      {report && report.protected.length > 0 && (
        <section className="card set-group cleanup-protected" aria-labelledby="cleanup-protected-title">
          <header className="card__head">
            <h2 id="cleanup-protected-title">Protected</h2>
            <span className="card__sub">not offered, and why</span>
          </header>
          <ul className="cleanup-protected__list">
            {report.protected.map((p) => (
              <li key={`${p.what}|${p.reason}`}>
                <span className="cleanup-protected__what">{p.what}</span>
                <span className="cleanup-protected__reason muted">{p.reason}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <CleanupHistory rows={history} error={historyError} groupLabel={groupLabel} />

      <CleanupPreviewDialog
        preview={preview}
        groupLabel={groupLabel}
        isBusy={phase === "deleting"}
        error={dialogError}
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          if (phase !== "deleting") setPreview(null);
        }}
      />
    </div>
  );
}
