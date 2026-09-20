import type { CleanupLogEntry } from "../../lib/ipc";
import { formatBytes } from "../../lib/units";
import { formatStamp } from "./cleanup-groups";

type Props = {
  rows: readonly CleanupLogEntry[];
  /** Why the history could not be loaded, if it could not. */
  error: string | null;
  groupLabel: (key: string) => string;
};

/** The last cleanup runs, newest first — one row per entry the core
 *  applied, as the `cleanup_log` table records them. Survives the deletion
 *  of what it names. */
export function CleanupHistory({ rows, error, groupLabel }: Props) {
  return (
    <section className="card set-group cleanup-history" aria-labelledby="cleanup-history-title">
      <header className="card__head">
        <h2 id="cleanup-history-title">Cleanup history</h2>
        <span className="card__sub">last {rows.length.toLocaleString()} entries</span>
      </header>
      {error && (
        <p className="settings__err" role="alert">
          Could not load the history: {error}
        </p>
      )}
      {!error && rows.length === 0 && <p className="muted">Nothing has been deleted from this page yet.</p>}
      {rows.length > 0 && (
        <div className="cleanup-history__scroll">
          <table className="cleanup-history__table">
            <thead>
              <tr>
                <th scope="col">When</th>
                <th scope="col">Group</th>
                <th scope="col">Entry</th>
                <th scope="col" className="numeric">
                  Files
                </th>
                <th scope="col" className="numeric">
                  Freed
                </th>
                <th scope="col" className="numeric">
                  Rows
                </th>
                <th scope="col" className="numeric">
                  Skipped
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.id}>
                  <td className="numeric">{formatStamp(r.ts, "datetime")}</td>
                  <td>{groupLabel(r.group_key)}</td>
                  <td className="cleanup-history__entry">{r.entry_label}</td>
                  <td className="numeric">{r.deleted_files.toLocaleString()}</td>
                  <td className="numeric">{formatBytes(r.freed_bytes)}</td>
                  <td className="numeric">{r.removed_rows.toLocaleString()}</td>
                  <td className="numeric">{r.skipped_count.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
