import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { createAgent, type AgentRuntime } from "../../lib/ipc";
import {
  autoPick,
  ctxLabel,
  HERMES_CTX_FLOOR,
  meetsHermesFloor,
  runtimeName,
  type CodingModel,
} from "./labels";

/** The "New profile" form. `open` lives in the parent so the "Start here"
 *  checklist can open it — the one control a first-time visitor needs. */
export function NewProfileForm({
  codingModels,
  runtimes,
  open,
  onOpenChange,
  onCreated,
}: {
  codingModels: readonly CodingModel[];
  runtimes: AgentRuntime[] | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const [adapter, setAdapter] = useState("opencode");
  const [name, setName] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [allowed, setAllowed] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const ids = useId();
  const nameId = `${ids}-name`;
  const runtimeId = `${ids}-runtime`;
  const modelFieldId = `${ids}-model`;
  const workspaceId = `${ids}-workspace`;
  const allowedId = `${ids}-allowed`;

  const rt = (runtimes ?? []).find((r) => r.id === adapter);
  const rtInstalled = rt?.installed ?? adapter === "opencode";
  const picked = modelId === "auto" ? null : codingModels.find((m) => m.id === modelId);
  const auto = autoPick(codingModels);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !workspace.trim() || busy || !rtInstalled) return;
    setBusy(true);
    setErr(null);
    try {
      await createAgent({
        name: name.trim(),
        adapter,
        model_id: modelId === "auto" ? null : modelId,
        workspace_path: workspace.trim(),
        allowed_paths: allowed
          .split(/[,\n]/)
          .map((s) => s.trim())
          .filter(Boolean),
      });
      setName("");
      setWorkspace("");
      setAllowed("");
      setModelId("auto");
      onOpenChange(false);
      onCreated();
    } catch (e2) {
      setErr(e2 instanceof Error ? e2.message : String(e2));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <button type="button" className="agents__addbtn" onClick={() => onOpenChange(true)}>
        + New profile
      </button>
    );
  }

  return (
    <form className="profform" onSubmit={submit} aria-label="New agent profile">
      <div className="profform__field">
        <span>
          <label htmlFor={nameId}>Name</label>
        </span>
        <input
          id={nameId}
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Repo coder"
          autoFocus
        />
      </div>

      <div className="profform__field">
        <span>
          <label htmlFor={runtimeId}>Runtime</label>
          <HelpHint area="agents" setting="runtime" describes={runtimeId} />
        </span>
        <select id={runtimeId} value={adapter} onChange={(e) => setAdapter(e.target.value)}>
          {(runtimes ?? [{ id: "opencode", installed: true }]).map((r) => (
            <option key={r.id} value={r.id}>
              {runtimeName(r.id)}
              {r.installed ? "" : " — not installed"}
            </option>
          ))}
        </select>
      </div>
      {!rtInstalled && (
        <p className="profform__warn">
          {runtimeName(adapter)} is not installed, so Create is off. Its card above has the
          install step.
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
              {m.name} — {ctxLabel(m.ctx_max)}
            </option>
          ))}
        </select>
      </div>
      {modelId === "auto" && auto && (
        <p className="muted">
          Auto would pick <strong>{auto.name}</strong> right now ({ctxLabel(auto.ctx_max)}) —
          whichever coding model was used last.
        </p>
      )}
      {adapter === "hermes" && picked && !meetsHermesFloor(picked) && (
        <p className="profform__warn">
          {picked.name} has a {ctxLabel(picked.ctx_max)}. Hermes needs{" "}
          {HERMES_CTX_FLOOR.toLocaleString("en-US")} tokens for its main model and its
          auxiliary compression model alike, and errors out below that. The profile saves;
          the session will not start.
        </p>
      )}
      {codingModels.length === 0 && (
        <p className="profform__warn">
          No model has the “coding” role yet — import a coding GGUF (Qwen2.5-Coder for
          OpenCode, Hermes-3 for Hermes) on the Models tab and tick “coding”.
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

      <div className="profform__field">
        <span>
          <label htmlFor={allowedId}>Extra read-only folders (optional, comma-separated)</label>
          <HelpHint area="agents" setting="extra-folders" describes={allowedId} />
        </span>
        <input
          id={allowedId}
          type="text"
          value={allowed}
          onChange={(e) => setAllowed(e.target.value)}
          placeholder="E:\\shared\\lib"
          spellCheck={false}
        />
      </div>

      <p className="muted">
        The agent may run shell commands and edit files — every command and every edit asks
        for your approval, edits stay inside the workspace folder, and it has no network
        access.
      </p>

      <div className="profform__buttons">
        <button
          type="submit"
          disabled={busy || !name.trim() || !workspace.trim() || !rtInstalled}
        >
          {busy ? "Creating…" : "Create profile"}
        </button>
        <button
          type="button"
          className="profform__cancel"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </button>
      </div>
      {err && <p className="agents__err">{err}</p>}
    </form>
  );
}
