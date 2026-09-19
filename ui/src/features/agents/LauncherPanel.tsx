import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { useAgentRuntimes, useLauncherStatus } from "../../lib/hooks";
import { launchExternal, stopExternalLaunch, type LaunchTool } from "../../lib/ipc";

const TOOL_LABEL: Record<LaunchTool, string> = {
  opencode: "OpenCode",
  hermes: "Hermes",
};

/** Opens a real, independent terminal running OpenCode or Hermes against a
 *  pinned local model -- distinct from the embedded agent sessions above:
 *  the user drives this terminal directly, and it keeps running even after
 *  AIWM closes (the backing model server does not -- see the warning below). */
export function LauncherPanel({
  codingModels,
}: {
  codingModels: { id: string; name: string }[];
}) {
  const { data: runtimes } = useAgentRuntimes();
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
    <section className="card launcher">
      <header className="card__head">
        <h2>Launch external terminal</h2>
      </header>
      <p className="muted">
        Opens a real, independent terminal window running the tool directly, pointed at a
        pinned local model. Unlike the sessions above, you drive it yourself and it keeps
        running even if you close AIWM -- but the model server does not, so closing AIWM
        leaves the terminal open with a dead connection.
      </p>

      {status ? (
        <div className="launcher__active">
          <div className="launcher__row">
            <span className="badge">{TOOL_LABEL[status.tool]}</span>
            <span>{status.model_name}</span>
            <span className="prof__path numeric">{status.workspace}</span>
          </div>
          {status.warning && <p className="agents__err">{status.warning}</p>}
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
            <select id={toolId} value={tool} onChange={(e) => setTool(e.target.value as LaunchTool)}>
              {(runtimes ?? [{ id: "opencode", installed: true }]).map((r) => (
                <option key={r.id} value={r.id}>
                  {TOOL_LABEL[r.id as LaunchTool] ?? r.id}
                  {r.installed ? "" : " — not installed"}
                </option>
              ))}
            </select>
          </div>
          {!rtInstalled && (
            <p className="muted">
              {TOOL_LABEL[tool]} isn’t installed — install it and reopen this form (see the
              profile setup above for Hermes).
            </p>
          )}

          <div className="profform__field">
            <span>
              <label htmlFor={modelFieldId}>Coding model</label>
              <HelpHint area="agents" setting="coding-model" describes={modelFieldId} />
            </span>
            <select id={modelFieldId} value={modelId} onChange={(e) => setModelId(e.target.value)}>
              <option value="auto">Auto — most-recently-used “coding” model</option>
              {codingModels.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
          </div>

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

          <button type="button" onClick={launch} disabled={busy || !workspace.trim() || !rtInstalled}>
            {busy ? "Launching…" : "Launch"}
          </button>
        </div>
      )}
      {err && <p className="agents__err">{err}</p>}
    </section>
  );
}
