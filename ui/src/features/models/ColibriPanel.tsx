import { useEffect, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { useRuntimes, useTelemetry } from "../../lib/hooks";
import {
  installColibri,
  listColibriModels,
  registerColibriModel,
  type ColibriModel,
} from "../../lib/ipc";
import { formatGB, formatGiB } from "../../lib/units";


/** Colibri (github.com/JustVugg/colibri) — a CPU-only engine for MoE models
 *  too large for GGUF/llama.cpp on a single consumer GPU. AIWM doesn't
 *  download the model itself (a few dozen safetensors shards; Hugging
 *  Face's own `hf` CLI fetches a directory like this far faster than this
 *  app's single-stream downloader would) — this panel shows the exact
 *  command to run, then registers wherever it lands. */
export function ColibriPanel() {
  const [models, setModels] = useState<ColibriModel[] | null>(null);
  useEffect(() => {
    listColibriModels().then(setModels).catch(() => setModels([]));
  }, []);
  const { data: runtimes } = useRuntimes();
  const { telemetry } = useTelemetry();

  const colibri = (runtimes ?? []).find((r) => r.id === "colibri");
  const installed = !!colibri && colibri.detail !== "not installed";
  const freeRamMb =
    telemetry?.host != null
      ? telemetry.host.ram_total_mb - telemetry.host.ram_used_mb
      : null;

  if (!models || models.length === 0) return null;

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Colibri (large local models)</h2>
        <span className="card__sub">
          {colibri?.detail ?? "not installed"} <HelpHint area="models" setting="colibri" />
        </span>
      </header>

      {!installed && <InstallRow />}

      {models.map((m) => (
        <ColibriRow key={m.id} model={m} freeRamMb={freeRamMb} disabled={!installed} />
      ))}
    </section>
  );
}

function InstallRow() {
  const [starting, setStarting] = useState(false);
  const install = async () => {
    setStarting(true);
    try {
      await installColibri();
    } catch {
      /* status shows the failure on the next poll */
    } finally {
      setStarting(false);
    }
  };
  return (
    <p className="colibri__setup">
      <span className="muted">
        Colibri runs models too large for your GPU on the CPU instead. Install the engine first
        (a small download — the model itself is separate, below).
      </span>{" "}
      <button type="button" className="chip" onClick={install} disabled={starting}>
        {starting ? "Starting…" : "Install Colibri"}
      </button>
    </p>
  );
}

function ColibriRow({
  model,
  freeRamMb,
  disabled,
}: {
  model: ColibriModel;
  freeRamMb: number | null;
  disabled: boolean;
}) {
  const [dir, setDir] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ kind: "ok" | "err"; text: string } | null>(null);
  const command = `hf download ${model.repo} --local-dir <path-you-choose>`;

  const ramTight = freeRamMb != null && freeRamMb < model.ram_estimate_mb;

  const copyCommand = async () => {
    try {
      await navigator.clipboard.writeText(command.replace("<path-you-choose>", dir || "D:\\models\\qwen36"));
    } catch {
      /* clipboard permission denied — the text is still selectable */
    }
  };

  const register = async () => {
    if (!dir.trim()) return;
    setBusy(true);
    setMessage(null);
    try {
      await registerColibriModel({ catalog_id: model.id, dir: dir.trim() });
      setMessage({ kind: "ok", text: "Registered — see it in the Model Library above." });
    } catch (err) {
      setMessage({ kind: "err", text: err instanceof Error ? err.message : String(err) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="colibri__row">
      <div className="colibri__head">
        <strong>{model.label}</strong>
        <span className="muted">
          {formatGB(model.disk_estimate_bytes)} download · needs ~{formatGiB(model.ram_estimate_mb)} RAM
          resident · {model.license}
        </span>
      </div>
      <p className="muted">{model.note}</p>
      {ramTight && (
        <p className="colibri__warn">
          Tight: only ~{formatGiB(freeRamMb ?? 0)} RAM free right now. This will still
          run — just expect more disk streaming (slower).
        </p>
      )}

      <ol className="colibri__steps">
        <li>
          Run this yourself (needs Python + <code>pip install -U "huggingface_hub[hf_transfer]"</code>{" "}
          once):
          <div className="colibri__cmd">
            <code>{command}</code>
            <button type="button" className="chip" onClick={copyCommand}>
              Copy
            </button>
          </div>
        </li>
        <li>
          Once it finishes, point AIWM at the folder you downloaded it into:
          <div className="colibri__register">
            <input
              type="text"
              value={dir}
              onChange={(e) => setDir(e.target.value)}
              placeholder="D:\models\qwen36"
              spellCheck={false}
              disabled={disabled}
            />
            <button type="button" onClick={register} disabled={disabled || busy || !dir.trim()}>
              {busy ? "Registering…" : "Register"}
            </button>
          </div>
        </li>
      </ol>
      {message && (
        <p className={message.kind === "ok" ? "colibri__ok" : "colibri__warn"}>{message.text}</p>
      )}
    </div>
  );
}
