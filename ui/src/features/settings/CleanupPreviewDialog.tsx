import { ConfirmDialog } from "../../components/ConfirmDialog";
import { skipReasonLabel } from "../../components/skip-reasons";
import type { CleanupApplyResult, CleanupEntryResult } from "../../lib/ipc";
import { formatBytes } from "../../lib/units";
import { countLabel, freedLabel, PREVIEW_PATH_CAP } from "./cleanup-groups";

type Props = {
  /** The dry run to show; `null` keeps the dialog closed. */
  preview: CleanupApplyResult | null;
  groupLabel: (key: string) => string;
  isBusy: boolean;
  /** The core's refusal of the real run, verbatim. */
  error: string | null;
  onConfirm: () => void;
  onCancel: () => void;
};

/** The dry run as a list: per entry, what goes and where, the paths capped
 *  with "… and N more"; the totals; the warning; Delete. Nothing has been
 *  deleted while this is open. */
export function CleanupPreviewDialog({
  preview,
  groupLabel,
  isBusy,
  error,
  onConfirm,
  onCancel,
}: Props) {
  const entries = preview?.entries ?? [];
  const items = entries.filter((e) => e.files > 0 || e.rows > 0).length;
  return (
    <ConfirmDialog
      isOpen={preview !== null}
      title="Delete what was found?"
      confirmLabel="Delete"
      tone="danger"
      size="wide"
      isBusy={isBusy}
      error={error}
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      {preview && (
        <>
          <p>
            <strong className="numeric">
              {freedLabel(preview.freed_bytes, preview.removed_rows, formatBytes)}
            </strong>{" "}
            will be freed from {countLabel(items, "item")} — {countLabel(preview.deleted_files, "file")}.
          </p>
          <ul className="cleanup-preview">
            {entries.map((e) => (
              <PreviewEntry key={`${e.group}|${e.id}`} entry={e} groupLabel={groupLabel(e.group)} />
            ))}
          </ul>
          <p className="confirm__warn">
            This deletes files permanently. Models, source media and anything a running job needs
            are never included.
          </p>
        </>
      )}
    </ConfirmDialog>
  );
}

function PreviewEntry({ entry, groupLabel }: { entry: CleanupEntryResult; groupLabel: string }) {
  const shown = entry.paths.slice(0, PREVIEW_PATH_CAP);
  const more = entry.paths.length - shown.length;
  const amount =
    entry.files > 0 || entry.rows > 0
      ? `${countLabel(entry.files, "file")}, ${freedLabel(entry.bytes, entry.rows, formatBytes)}`
      : "nothing to delete";
  return (
    <li className="cleanup-preview__entry">
      <div className="cleanup-preview__head">
        <span className="cleanup-preview__label">{entry.label}</span>
        <span className="cleanup-preview__group muted">{groupLabel}</span>
        <span className="cleanup-preview__amount numeric">{amount}</span>
      </div>
      {shown.length > 0 && (
        <ul className="cleanup-preview__paths">
          {shown.map((p) => (
            <li key={p}>{p}</li>
          ))}
          {more > 0 && <li className="cleanup-preview__more">… and {more.toLocaleString()} more</li>}
        </ul>
      )}
      {entry.skipped.length > 0 && (
        <ul className="cleanup-preview__skipped">
          {entry.skipped.map((s) => (
            <li key={`${s.path}|${s.reason}`}>
              Skipped <span className="cleanup-preview__path">{s.path}</span> — {skipReasonLabel(s)}
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}
