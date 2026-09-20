import { useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import {
  cancelDownload,
  clearFinishedDownloads,
  deleteDownload,
  pauseDownload,
  resumeDownload,
  type Download,
  type DownloadState,
} from "../../lib/ipc";
import { useDownloads } from "../../lib/hooks";
import { formatGB } from "../../lib/units";

const pct = (d: Download) =>
  d.size_bytes && d.size_bytes > 0 ? Math.min(100, (d.bytes_done / d.size_bytes) * 100) : null;

const STATE_LABEL: Record<DownloadState, string> = {
  queued: "Queued",
  running: "Downloading",
  paused: "Paused",
  verifying: "Verifying…",
  done: "Imported",
  failed: "Failed",
};

const TERMINAL: DownloadState[] = ["done", "failed"];

/** Active + recent model downloads. Hidden when the list is empty. History
 *  (done/failed rows) can pile up once you've deleted the models those
 *  downloads brought in -- "Clear finished" wipes just those, in one go. */
export function Downloads() {
  const { data, refetch } = useDownloads();
  const rows = data ?? [];
  const [clearing, setClearing] = useState(false);
  if (rows.length === 0) return null;

  const finishedCount = rows.filter((d) => TERMINAL.includes(d.state)).length;

  const clearFinished = async () => {
    setClearing(true);
    try {
      await clearFinishedDownloads();
      refetch();
    } catch {
      /* the list just won't shrink -- no need for a modal over this */
    } finally {
      setClearing(false);
    }
  };

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Downloads</h2>
        <span className="card__sub">
          one at a time · verified, then imported <HelpHint area="models" setting="downloads" />
        </span>
        {finishedCount > 0 && (
          <button type="button" className="chip" onClick={clearFinished} disabled={clearing}>
            {clearing ? "Clearing…" : `Clear finished (${finishedCount})`}
          </button>
        )}
      </header>
      <ul className="dl__list">
        {rows.map((d) => (
          <DownloadRow key={d.id} d={d} onRemoved={refetch} />
        ))}
      </ul>
    </section>
  );
}

function DownloadRow({ d, onRemoved }: { d: Download; onRemoved: () => void }) {
  const [removing, setRemoving] = useState(false);
  const remove = async () => {
    setRemoving(true);
    try {
      await deleteDownload(d.id);
      onRemoved();
    } catch {
      setRemoving(false);
    }
  };
  const p = pct(d);
  const done = d.bytes_done;
  const size = d.size_bytes ?? 0;

  return (
    <li className={`dl__row dl__row--${d.state}`}>
      <div className="dl__main">
        <span className="dl__name">{d.filename}</span>
        <span className="dl__meta numeric">
          {STATE_LABEL[d.state]}
          {(d.state === "running" || d.state === "paused") &&
            ` · ${formatGB(done, 2)}${size ? ` / ${formatGB(size, 2)}` : ""}`}
          {d.state === "failed" && d.error_text && ` · ${d.error_text}`}
          {d.retries > 0 && d.state !== "done" && ` · retry ${d.retries}`}
        </span>
        {p != null && (d.state === "running" || d.state === "paused") && (
          <div className="dl__bar">
            <div className="dl__bar-fill" style={{ width: `${p}%` }} />
          </div>
        )}
      </div>
      <div className="known__actions">
        {d.state === "running" && (
          <button type="button" onClick={() => pauseDownload(d.id).catch(() => {})}>
            Pause
          </button>
        )}
        {(d.state === "paused" || d.state === "failed") && (
          <button type="button" onClick={() => resumeDownload(d.id).catch(() => {})}>
            Resume
          </button>
        )}
        {d.state !== "done" && d.state !== "failed" && (
          <button type="button" onClick={() => cancelDownload(d.id).catch(() => {})}>
            Cancel
          </button>
        )}
        {TERMINAL.includes(d.state) && (
          <button type="button" onClick={remove} disabled={removing}>
            {removing ? "…" : "Remove"}
          </button>
        )}
      </div>
    </li>
  );
}
