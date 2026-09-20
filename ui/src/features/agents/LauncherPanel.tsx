import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { useLauncherStatus } from "../../lib/hooks";
import {
  launchExternal,
  stopExternalLaunch,
  type AgentRuntime,
  type LaunchTool,
} from "../../lib/ipc";
import {
  ctxLabel,
  folderName,
  HERMES_CTX_FLOOR,
  meetsHermesFloor,
  runtimeName,
  type CodingModel,
} from "./labels";

/** Opens a real, independent terminal running OpenCode or Hermes against a
 *  pinned local model — distinct from the embedded sessions above: the person
 *  drives that terminal directly, and it keeps running after AIWM closes (the
 *  backing model server does not, which is the warning below). Deliberately
 *  the quietest card on the tab: it is the alternative route, not the one to
 *  take first. */
export function LauncherPanel({
  codingModels,
  runtimes,
}: {
  codingModels: readonly CodingModel[];
  runtimes: AgentRuntime[] | null;
}) {
  const { data: status, refetch } = useLauncherStatus();

  const [tool, setTool] = useState<LaunchTool>("opencode");
  const [modelId, setModelId] = useState("auto");
  const [workspace, setWorkspace] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const ids = useId();
  const toolId = `${ids}-tool`;
  const modelFieldId = `${ids}-model`;
  const workspaceId = `${ids}-workspace`;

  const rt = (runtimes ?? []).find((r) => r.id === tool);
  const rtInstalled = rt?.installed ?? tool === "opencode";
  const picked = modelId === "auto" ? null : codingModels.find((m) => m.id === modelId);
  const ctxRisk = tool === "hermes" && picked != null && !meetsHermesFloor(picked);

  const launch = async () => {
    if (!workspace.trim() || busy || !rtInstalled) return;
    setBusy(true);
    setErr(null);
    try {
      await launchExternal({
        tool,
        model_id: modelId === "auto" ? null : modelId,
        workspace: workspace.trim(),
      });
      refetch();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await stopExternalLaunch();
      refetch();
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card launcher" aria-labelledby="agents-launch-h">
      <header className="card__head">
        <h2 id="agents-launch-h">Or: the tool in its own terminal</h2>
        <span className="card__sub">
          <HelpHint area="agents" setting="launcher" />
        </span>
      </header>
      <p className="muted launcher__lede">
        Opens a real terminal window running the tool directly against a pinned local model.
        You drive it yourself — no transcript, no approval prompts here — and it keeps running
        after AIWM closes. The model server does not, so closing AIWM leaves that terminal
        with a dead connection.
      </p>

      <p className="visually-hidden" aria-live="polite">
        {status
          ? `${runtimeName(status.tool)} is running in ${status.workspace} on ${status.model_name}.`
          : "No external terminal is running."}
      </p>

      {status ? (
        <div className="launcher__active">
          <p className="launcher__live">
            <span className="launcher__dot" aria-hidden="true" />
            <strong>{runtimeName(status.tool)} is running</strong> in{" "}
            {folderName(status.workspace)}, on {status.model_name}.
          </p>
          <dl className="launcher__facts">
            <dt>Folder</dt>
            <dd className="numeric">{status.workspace}</dd>
            <dt>Model served at</dt>
            <dd className="numeric">{status.base_url}</dd>
          </dl>
          {status.warning && <p className="launcher__warn">{status.warning}</p>}
          <button type="button" onClick={stop} disabled={busy}>
            {busy ? "Releasing…" : "Stop & release model"}
          </button>
        </div>
      ) : (
        <div className="launcher__form">
          <div className="profform__field">
            <span>
              <label htmlFor={toolId}>Tool</label>
              <HelpHint area="agents" setting="launcher" describes={toolId} />
            </span>
            <select
              id={toolId}
              value={tool}
              onChange={(e) => setTool(e.target.value as LaunchTool)}
            >
              {(runtimes ?? [{ id: "opencode", installed: true }]).map((r) => (
                <option key={r.id} value={r.id}>
                  {runtimeName(r.id)}
                  {r.installed ? "" : " — not installed"}
                </option>
              ))}
            </select>
          </div>
          {!rtInstalled && (
            <p className="launcher__warn">
              {runtimeName(tool)} is not installed, so Launch is off — its card above has the
              install step.
            </p>
          )}

          <div className="profform__field">
            <span>
              <label htmlFor={modelFieldId}>Coding model</label>
              <HelpHint area="agents" setting="coding-model" describes={modelFieldId} />
            </span>
            <select
              id={modelFieldId}
              value={modelId}
              onChange={(e) => setModelId(e.target.value)}
            >
              <option value="auto">Auto — most-recently-used “coding” model</option>
              {codingModels.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name} — {ctxLabel(m.ctx_max)}
                </option>
              ))}
            </select>
          </div>
          {ctxRisk && picked && (
            <p className="launcher__warn">
              {picked.name} has a {ctxLabel(picked.ctx_max)}; Hermes needs{" "}
              {HERMES_CTX_FLOOR.toLocaleString("en-US")} tokens. The terminal opens, then the
              tool errors out.
            </p>
          )}

          <div className="profform__field">
            <span>
              <label htmlFor={workspaceId}>Workspace folder</label>
              <HelpHint area="agents" setting="workspace" describes={workspaceId} />
            </span>
            <input
              id={workspaceId}
              type="text"
              value={workspace}
              onChange={(e) => setWorkspace(e.target.value)}
              placeholder="E:\\projects\\my-repo"
              spellCheck={false}
            />
          </div>

          <button
            type="button"
            onClick={launch}
            disabled={busy || !workspace.trim() || !rtInstalled}
          >
            {busy ? "Launching…" : "Launch terminal"}
          </button>
        </div>
      )}
      {err && <p className="agents__err">{err}</p>}
    </section>
  );
}
