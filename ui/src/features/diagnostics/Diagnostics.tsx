import { useEffect, useRef, useState } from "react";
import {
  useAbout,
  useExternalEngines,
  useLogs,
  useRegistryStatus,
  useRuntimes,
  useTelemetry,
} from "../../lib/hooks";
import {
  attachExternalEngine,
  checkToolVersions,
  detachEngine,
  installComfyui,
  installLlamacpp,
  unloadModel,
  type RuntimeStatus,
  type ToolUpdateStatus,
  type ToolVersionCheck,
} from "../../lib/ipc";
import { getTheme } from "../../lib/theme";
import "./diagnostics.css";

export function Diagnostics() {
  const { data: runtimes, refetch: refetchRuntimes } = useRuntimes();
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
          <UnloadAllButton runtimes={runtimes} onUnloaded={refetchRuntimes} />
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

      <ToolUpdatesCard />

      <ExternalEngineCard llama={runtimes?.find((r) => r.id === "llamacpp")} />

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

const TOOL_LABELS: Record<ToolVersionCheck["id"], string> = {
  comfyui: "ComfyUI",
  llamacpp: "llama.cpp",
  colibri: "Colibri",
  hermes: "Hermes",
  opencode: "OpenCode",
};

function describeToolStatus(status: ToolUpdateStatus): string {
  if (status.state === "up_to_date") return "up to date";
  if (status.state === "update_available") return `${status.latest} available`;
  if (status.state === "unmanaged") return `${status.latest} available upstream`;
  return `check failed — ${status.error}`;
}

/** Drives the row's `[data-state]` color accent — amber for an update worth
 *  taking, red for a failed check, unset (neutral) otherwise. */
function toolStatusDataState(status: ToolUpdateStatus): "warn" | "crit" | undefined {
  if (status.state === "update_available") return "warn";
  if (status.state === "check_failed") return "crit";
  return undefined;
}

/** Deterministic "is AIWM's pinned version behind upstream" check for the
 *  five externally-sourced tools (ComfyUI, llama.cpp, Colibri, Hermes,
 *  OpenCode) — a plain version-string compare against GitHub Releases / PyPI,
 *  never the LLM-driven per-model advisor elsewhere in the app. Manually
 *  triggered rather than polled: it makes a handful of real upstream HTTP
 *  calls every time. */
function ToolUpdatesCard() {
  const [results, setResults] = useState<ToolVersionCheck[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const check = async () => {
    setBusy(true);
    setError(null);
    try {
      setResults(await checkToolVersions());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card">
      <header className="card__head">
        <h2>Tool updates</h2>
        <button type="button" className="diag__copy" onClick={check} disabled={busy}>
          {busy ? "Checking…" : "Check for updates"}
        </button>
      </header>
      {results ? (
        <table className="rt">
          <tbody>
            {results.map((r) => (
              <tr key={r.id}>
                <td>{TOOL_LABELS[r.id]}</td>
                <td className="muted numeric">{r.current ?? "bring your own"}</td>
                <td className="muted" data-state={toolStatusDataState(r.status)}>
                  {describeToolStatus(r.status)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p className="muted">
          {error ??
            "Compares ComfyUI, llama.cpp, Colibri and Hermes' pinned version against the latest one published upstream (OpenCode has no AIWM-managed pin)."}
        </p>
      )}
      {results && error && <span className="muted">{error}</span>}
    </section>
  );
}

/** Bring-your-own-engine (7.x): attach an already-running local LLM server
 *  (Ollama, LM Studio, a standalone llama-server, …) instead of installing
 *  AIWM's own — Chat and Agents use it exactly the same way afterward, since
 *  it lands in the same llama.cpp runtime slot a self-managed server would. */
function ExternalEngineCard({ llama }: { llama: RuntimeStatus | undefined }) {
  const { data: engines } = useExternalEngines();
  const [vramMb, setVramMb] = useState("4096");
  const [customPort, setCustomPort] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const attachedModelId = llama?.loaded_models[0]?.model_id ?? null;
  const isExternal = llama?.detail?.startsWith("attached to") ?? false;

  const attach = async (port: number, modelId: string) => {
    const vram = Number(vramMb);
    if (!Number.isFinite(vram) || vram <= 0) {
      setMsg("Enter a VRAM estimate in MB first.");
      return;
    }
    if (!Number.isFinite(port) || port <= 0) {
      setMsg("Enter a valid port.");
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      await attachExternalEngine(port, modelId, Math.round(vram));
      setMsg(`Attached to ${modelId} on :${port}.`);
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const detach = async () => {
    if (!attachedModelId) return;
    setBusy(true);
    setMsg(null);
    try {
      await detachEngine(attachedModelId);
      setMsg("Detached.");
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card">
      <header className="card__head">
        <h2>Bring your own engine</h2>
        <span className="card__sub">attaches, doesn't install</span>
      </header>
      <p className="muted">
        Already running Ollama or LM Studio? Attach it instead of a self-managed
        llama-server — its process is never touched by AIWM.
      </p>

      {isExternal && (
        <div className="rt-setup">
          <span className="muted">{llama?.detail}</span>
          <button type="button" disabled={busy} onClick={detach}>
            Detach
          </button>
        </div>
      )}

      {engines && engines.length > 0 ? (
        <table className="rt">
          <tbody>
            {engines.flatMap((e) =>
              e.models.map((m) => (
                <tr key={`${e.port}-${m}`}>
                  <td>{e.label}</td>
                  <td className="muted">{m}</td>
                  <td className="muted numeric">:{e.port}</td>
                  <td>
                    <button type="button" disabled={busy} onClick={() => attach(e.port, m)}>
                      Attach
                    </button>
                  </td>
                </tr>
              )),
            )}
          </tbody>
        </table>
      ) : (
        <p className="muted">Nothing found on the usual ports (Ollama :11434, LM Studio :1234).</p>
      )}

      <label className="set-field">
        <span>
          VRAM estimate (MB) — used for the scheduler's budget math, since AIWM can't
          inspect a process it doesn't manage
        </span>
        <input
          type="text"
          inputMode="numeric"
          value={vramMb}
          onChange={(e) => setVramMb(e.target.value)}
        />
      </label>

      <details>
        <summary className="muted">Attach a custom address (127.0.0.1 only)</summary>
        <div className="rt-setup">
          <input
            type="text"
            placeholder="port, e.g. 11434"
            value={customPort}
            onChange={(e) => setCustomPort(e.target.value)}
          />
          <input
            type="text"
            placeholder="model id"
            value={customModel}
            onChange={(e) => setCustomModel(e.target.value)}
          />
          <button
            type="button"
            disabled={busy || !customPort.trim() || !customModel.trim()}
            onClick={() => attach(Number(customPort), customModel.trim())}
          >
            Attach
          </button>
        </div>
      </details>

      {msg && <span className="muted">{msg}</span>}
    </section>
  );
}

/** Free every resident model at once — loops the same per-model unload the
 *  Dashboard's own UnloadButton calls, one request per loaded model. */
function UnloadAllButton({
  runtimes,
  onUnloaded,
}: {
  runtimes: RuntimeStatus[] | null;
  onUnloaded: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const modelIds = (runtimes ?? []).flatMap((r) => r.loaded_models.map((m) => m.model_id));

  const click = async () => {
    setBusy(true);
    setErr(null);
    const failures: string[] = [];
    for (const id of modelIds) {
      try {
        await unloadModel(id);
      } catch (e) {
        failures.push(e instanceof Error ? e.message : String(e));
      }
    }
    setBusy(false);
    setErr(failures.length > 0 ? failures.join("; ") : null);
    onUnloaded();
  };

  return (
    <span className="diag__unload-wrap">
      <button
        type="button"
        className="diag__unload-btn"
        onClick={click}
        disabled={busy || modelIds.length === 0}
        title="Free every resident model's VRAM/RAM right now"
      >
        {busy ? "Unloading…" : "Unload all models"}
      </button>
      {err && <span className="diag__unload-err">{err}</span>}
    </span>
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
