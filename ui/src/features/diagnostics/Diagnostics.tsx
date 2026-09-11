import { useEffect, useRef, useState } from "react";
import {
  useAbout,
  useLogs,
  useRegistryStatus,
  useRuntimes,
  useTelemetry,
} from "../../lib/hooks";
import { installComfyui, installLlamacpp } from "../../lib/ipc";
import { getTheme } from "../../lib/theme";
import "./diagnostics.css";

export function Diagnostics() {
  const { data: runtimes } = useRuntimes();
  const { data: logs } = useLogs();
  const { telemetry } = useTelemetry();
  const { data: registry } = useRegistryStatus();
  const about = useAbout();
  const logRef = useRef<HTMLPreElement>(null);
  const detailOf = (id: string) => runtimes?.find((r) => r.id === id)?.detail ?? null;
  const gpu = telemetry?.gpu;
  const processes = gpu?.state === "available" ? gpu.processes : [];

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const copyEnvironment = () => {
    const lines = [
      `core         ${about?.core_version ?? "?"}`,
      `data dir     ${about?.data_dir ?? "?"}`,
      `store        ${about?.store_path ?? "?"}`,
      `outputs      ${about?.outputs_dir ?? "?"}`,
      `runtimes     ${about?.runtimes_dir ?? "?"}`,
      `cache        ${about?.cache_dir ?? "?"}`,
      `API port     127.0.0.1:${about?.core_api_port ?? "?"}`,
      `VRAM budget  ${about?.vram_budget_mb ?? "?"} MB`,
      `offline      ${about ? String(about.offline_mode) : "?"}`,
      `theme        ${getTheme()}`,
      `gpu          ${gpu?.state === "available" ? gpu.name : "unavailable"}`,
    ];
    void navigator.clipboard?.writeText(lines.join("\n"));
  };

  return (
    <div className="diag">
      <section className="card">
        <header className="card__head">
          <h2>Environment</h2>
          <button type="button" className="diag__copy" onClick={copyEnvironment}>
            Copy
          </button>
        </header>
        <dl className="kv numeric">
          <dt>core</dt>
          <dd>{about?.core_version ?? "…"}</dd>
          <dt>data dir</dt>
          <dd>{about?.data_dir ?? "…"}</dd>
          <dt>store</dt>
          <dd>{about?.store_path ?? "…"}</dd>
          <dt>outputs</dt>
          <dd>{about?.outputs_dir ?? "…"}</dd>
          <dt>runtimes</dt>
          <dd>{about?.runtimes_dir ?? "…"}</dd>
          <dt>cache</dt>
          <dd>{about?.cache_dir ?? "…"}</dd>
          <dt>API port</dt>
          <dd>{about ? `127.0.0.1:${about.core_api_port}` : "…"}</dd>
          <dt>VRAM budget</dt>
          <dd>{about ? `${about.vram_budget_mb} MB` : "…"}</dd>
          <dt>offline</dt>
          <dd>{about ? String(about.offline_mode) : "…"}</dd>
          <dt>theme</dt>
          <dd>{getTheme()}</dd>
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
        <RuntimeSetup
          detail={detailOf("llamacpp")}
          label="llama.cpp"
          sizeHint="about 645 MB"
          install={installLlamacpp}
        />
        <RuntimeSetup
          detail={detailOf("comfyui")}
          label="ComfyUI"
          sizeHint="several GB — PyTorch is a large download"
          install={installComfyui}
        />
      </section>

      <section className="card card--wide">
        <header className="card__head">
          <h2>GPU processes</h2>
          <span className="card__sub">
            {gpu?.state === "available"
              ? `${gpu.vram_used_mb} / ${gpu.vram_total_mb} MB in use`
              : ""}
          </span>
        </header>
        {gpu?.state !== "available" ? (
          <p className="muted">No GPU telemetry.</p>
        ) : processes.length === 0 ? (
          <p className="muted">Nothing is holding VRAM right now.</p>
        ) : (
          <table className="rt numeric">
            <tbody>
              {processes.map((p) => (
                <tr key={p.pid}>
                  <td>PID {p.pid}</td>
                  <td className="muted">{p.vram_mb} MB</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className="card">
        <header className="card__head">
          <h2>Model registry</h2>
          <span className="card__sub">{registry?.source_id ?? "…"}</span>
        </header>
        <dl className="kv numeric">
          <dt>last fetch</dt>
          <dd>{registry?.last_fetch ? registry.last_fetch.replace("T", " ").slice(0, 19) : "—"}</dd>
          <dt>cache entries</dt>
          <dd>{registry?.cache_entries ?? "…"}</dd>
          <dt>rate limit left</dt>
          <dd>
            {registry?.rate_limited_secs
              ? `backing off ${registry.rate_limited_secs}s`
              : (registry?.rate_limit_remaining ?? "—")}
          </dd>
          <dt>HF token</dt>
          <dd>{registry ? (registry.token_set ? "set" : "not set") : "…"}</dd>
        </dl>
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

const INSTALL_PREFIXES = ["downloading", "extracting", "unpacking", "creating", "installing"];
const isInstalling = (d: string | null) =>
  d != null && INSTALL_PREFIXES.some((p) => d.startsWith(p));

type RuntimeSetupProps = {
  detail: string | null;
  label: string;
  sizeHint: string;
  install: () => Promise<string>;
};

/** The "Set up <runtime>" affordance — shown only when a runtime is missing or
 *  its last setup failed, plus the live progress line while it runs. */
function RuntimeSetup({ detail, label, sizeHint, install }: RuntimeSetupProps) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const installing = isInstalling(detail);
  const actionable = detail === "not installed" || (detail?.startsWith("setup failed") ?? false);

  if (!installing && !actionable) return null;

  const start = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const status = await install();
      setMessage(
        status === "already_installed"
          ? "Already installed."
          : `Setup started — ${sizeHint}. This runs in the background.`,
      );
    } catch (err) {
      setMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rt-setup">
      <button type="button" onClick={start} disabled={busy || installing}>
        {installing ? "Setting up…" : busy ? "Starting…" : `Set up ${label}`}
      </button>
      {installing && detail && <span className="muted">{detail}</span>}
      {message && <span className="muted">{message}</span>}
    </div>
  );
}
