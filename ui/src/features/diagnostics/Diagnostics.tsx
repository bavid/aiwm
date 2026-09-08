import { useEffect, useRef } from "react";
import { useAbout, useLogs, useRuntimes } from "../../lib/hooks";
import "./diagnostics.css";

export function Diagnostics() {
  const { data: runtimes } = useRuntimes();
  const { data: logs } = useLogs();
  const about = useAbout();
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs]);

  return (
    <div className="diag">
      <section className="card">
        <header className="card__head">
          <h2>Environment</h2>
        </header>
        <dl className="kv numeric">
          <dt>core</dt>
          <dd>{about?.core_version ?? "…"}</dd>
          <dt>data dir</dt>
          <dd>{about?.data_dir ?? "…"}</dd>
          <dt>store</dt>
          <dd>{about?.store_path ?? "…"}</dd>
          <dt>API port</dt>
          <dd>{about ? `127.0.0.1:${about.core_api_port}` : "…"}</dd>
          <dt>offline</dt>
          <dd>{about ? String(about.offline_mode) : "…"}</dd>
        </dl>
      </section>

      <section className="card">
        <header className="card__head">
          <h2>Runtimes</h2>
        </header>
        {runtimes && runtimes.length > 0 ? (
          <table className="rt">
            <tbody>
              {runtimes.map((r) => (
                <tr key={r.id}>
                  <td>
                    <span className="status-dot" data-state={r.health} /> {r.id}
                  </td>
                  <td className="muted">{r.detail ?? r.kind}</td>
                  <td className="muted numeric">{r.vram_used_mb} MB</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="muted">No runtimes registered yet.</p>
        )}
      </section>

      <section className="card card--wide">
        <header className="card__head">
          <h2>Log</h2>
          <span className="card__sub">{about ? "aiwm.log" : ""}</span>
        </header>
        <pre className="log" ref={logRef}>
          {logs?.join("\n") ?? "…"}
        </pre>
      </section>
    </div>
  );
}
