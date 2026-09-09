import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { exportBackup, importBackup, type ImportSummary } from "../../lib/ipc";

const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Export / restore a portable backup. The archive carries a database snapshot,
 *  `config.toml` and a model manifest (names + hashes) — never the model files
 *  themselves. A restore stages the archive and takes effect on the next start. */
export function BackupCard() {
  const [exporting, setExporting] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  const [exportErr, setExportErr] = useState<string | null>(null);

  const [path, setPath] = useState("");
  const [restoring, setRestoring] = useState(false);
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const [restoreErr, setRestoreErr] = useState<string | null>(null);

  const runExport = async () => {
    setExporting(true);
    setExported(null);
    setExportErr(null);
    try {
      setExported(await exportBackup());
    } catch (e) {
      setExportErr(errText(e));
    } finally {
      setExporting(false);
    }
  };

  const runRestore = async () => {
    if (!path.trim() || restoring) return;
    setRestoring(true);
    setSummary(null);
    setRestoreErr(null);
    try {
      setSummary(await importBackup(path.trim()));
    } catch (e) {
      setRestoreErr(errText(e));
    } finally {
      setRestoring(false);
    }
  };

  return (
    <section className="card set-group">
      <header className="card__head">
        <h2>Backup &amp; restore</h2>
        <span className="card__sub">manual</span>
      </header>
      <p className="muted">
        A portable <code>.zip</code>: a database snapshot (agent profiles,
        sessions, transcripts, model records, jobs), <code>config.toml</code> and
        a model manifest — <strong>not</strong> the model files. Restore stages
        the archive; it takes effect after a restart, keeping the replaced files
        as <code>*.pre-import</code>.
      </p>

      <div className="backup-row">
        <button
          type="button"
          className="backup-btn"
          disabled={exporting}
          onClick={runExport}
        >
          {exporting ? "Exporting…" : "Export backup"}
        </button>
        {exported && (
          <span className="backup-path">
            <code>{exported}</code>
            <button
              type="button"
              className="set-reveal"
              onClick={() => revealItemInDir(exported).catch(() => {})}
            >
              reveal
            </button>
          </span>
        )}
      </div>
      {exportErr && <p className="settings__err">{exportErr}</p>}

      <label className="set-field">
        <span>Restore from an export archive — full path to the <code>.zip</code></span>
        <input
          type="text"
          value={path}
          spellCheck={false}
          placeholder="E:\AI\data\exports\aiwm-export-….zip"
          onChange={(e) => {
            setPath(e.target.value);
            setSummary(null);
            setRestoreErr(null);
          }}
        />
      </label>
      <div className="backup-row">
        <button
          type="button"
          className="backup-btn"
          disabled={!path.trim() || restoring}
          onClick={runRestore}
        >
          {restoring ? "Staging…" : "Restore"}
        </button>
      </div>
      {restoreErr && <p className="settings__err">{restoreErr}</p>}

      {summary && (
        <div className="backup-summary">
          <p className="settings__ok">
            Archive staged. <strong>Restart AI Workstation Manager</strong> to
            finish the restore.
          </p>
          <dl className="set-kv">
            <dt>Written by</dt>
            <dd>core {summary.core_version}</dd>
            <dt>Created</dt>
            <dd>{summary.created_at}</dd>
            <dt>Models in archive</dt>
            <dd className="numeric">{summary.model_count}</dd>
          </dl>
          {summary.missing_models.length > 0 && (
            <>
              <p className="muted">
                {summary.missing_models.length === 1
                  ? "1 model referenced by the archive is"
                  : `${summary.missing_models.length} models referenced by the archive are`}{" "}
                not in this machine's store — re-import them after the restart:
              </p>
              <ul className="backup-missing">
                {summary.missing_models.map((m) => (
                  <li key={m}>{m}</li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}
