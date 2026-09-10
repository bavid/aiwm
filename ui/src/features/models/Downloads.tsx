import {
  cancelDownload,
  pauseDownload,
  resumeDownload,
  type Download,
  type DownloadState,
} from "../../lib/ipc";
import { useDownloads } from "../../lib/hooks";

const gb = (n: number) => `${(n / 1024 ** 3).toFixed(2)} GB`;
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

/** Active + recent model downloads. Hidden when the list is empty. */
export function Downloads() {
  const { data } = useDownloads();
  const rows = data ?? [];
  if (rows.length === 0) return null;

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Downloads</h2>
        <span className="card__sub">one at a time · verified, then imported</span>
      </header>
      <ul className="dl__list">
        {rows.map((d) => (
          <DownloadRow key={d.id} d={d} />
        ))}
      </ul>
    </section>
  );
}

function DownloadRow({ d }: { d: Download }) {
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
            ` · ${gb(done)}${size ? ` / ${gb(size)}` : ""}`}
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
        {d.state !== "done" && (
          <button type="button" onClick={() => cancelDownload(d.id).catch(() => {})}>
            Cancel
          </button>
        )}
      </div>
    </li>
  );
}
