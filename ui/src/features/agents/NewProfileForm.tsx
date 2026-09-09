import { useState } from "react";
import { useAgentRuntimes } from "../../lib/hooks";
import { createAgent, installHermes, type AgentInstallStatus } from "../../lib/ipc";

const RUNTIME_LABEL: Record<string, string> = {
  opencode: "OpenCode",
  hermes: "Hermes",
};

/** The collapsible "New profile" form. `codingModels` are the library models
 *  carrying the `coding` role. */
export function NewProfileForm({
  codingModels,
  onCreated,
}: {
  codingModels: { id: string; name: string }[];
  onCreated: () => void;
}) {
  const { data: runtimes } = useAgentRuntimes();
  const [open, setOpen] = useState(false);
  const [adapter, setAdapter] = useState("opencode");
  const [name, setName] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [allowed, setAllowed] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const rt = (runtimes ?? []).find((r) => r.id === adapter);
  const rtInstalled = rt?.installed ?? adapter === "opencode";

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
      setOpen(false);
      onCreated();
    } catch (e2) {
      setErr(e2 instanceof Error ? e2.message : String(e2));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <button type="button" className="agents__addbtn" onClick={() => setOpen(true)}>
        + New profile
      </button>
    );
  }

  return (
    <form className="profform" onSubmit={submit}>
      <label className="profform__field">
        <span>Name</span>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Repo coder"
        />
      </label>

      <label className="profform__field">
        <span>Runtime</span>
        <select value={adapter} onChange={(e) => setAdapter(e.target.value)}>
          {(runtimes ?? [{ id: "opencode", installed: true }]).map((r) => (
            <option key={r.id} value={r.id}>
              {RUNTIME_LABEL[r.id] ?? r.id}
              {r.installed ? "" : " — not installed"}
            </option>
          ))}
        </select>
      </label>
      {!rtInstalled && rt && <RuntimeSetup runtime={rt} />}

      <label className="profform__field">
        <span>Coding model</span>
        <select value={modelId} onChange={(e) => setModelId(e.target.value)}>
          <option value="auto">Auto — most-recently-used “coding” model</option>
          {codingModels.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
      </label>
      {codingModels.length === 0 && (
        <p className="muted">
          No model has the “coding” role yet — import a coding GGUF (e.g. Qwen2.5-Coder for
          OpenCode, Hermes-3 for Hermes) on the Models tab and tick “coding”.
        </p>
      )}

      <label className="profform__field">
        <span>Workspace folder</span>
        <input
          type="text"
          value={workspace}
          onChange={(e) => setWorkspace(e.target.value)}
          placeholder="E:\\projects\\my-repo"
          spellCheck={false}
        />
      </label>

      <label className="profform__field">
        <span>Extra read-only folders (optional, comma-separated)</span>
        <input
          type="text"
          value={allowed}
          onChange={(e) => setAllowed(e.target.value)}
          placeholder="E:\\shared\\lib"
          spellCheck={false}
        />
      </label>

      <p className="muted">
        The agent may run shell commands and edit files — every command and every edit asks for
        your approval, edits stay inside the workspace, and it has no network access.
      </p>

      <div className="profform__buttons">
        <button
          type="submit"
          disabled={busy || !name.trim() || !workspace.trim() || !rtInstalled}
        >
          {busy ? "Creating…" : "Create profile"}
        </button>
        <button type="button" className="profform__cancel" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
      {err && <p className="agents__err">{err}</p>}
    </form>
  );
}

function phaseLabel(s: AgentInstallStatus): string {
  if (s.state === "running") {
    const pct =
      s.total_bytes > 0 ? ` ${Math.round((s.done_bytes / s.total_bytes) * 100)}%` : "";
    return `${s.phase.replace(/_/g, " ")}${pct}…`;
  }
  if (s.state === "failed") return `failed: ${s.error}`;
  return "";
}

function RuntimeSetup({
  runtime,
}: {
  runtime: { id: string; install?: AgentInstallStatus };
}) {
  const [starting, setStarting] = useState(false);
  const status = runtime.install;
  const running = status?.state === "running";

  if (runtime.id !== "hermes") {
    return (
      <p className="muted">
        OpenCode isn’t installed. Install <code>opencode-ai</code> (npm) and make sure{" "}
        <code>opencode</code> is on your PATH, then reopen this form.
      </p>
    );
  }

  const install = async () => {
    setStarting(true);
    try {
      await installHermes();
    } catch {
      /* status will show the failure on the next poll */
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="rtsetup">
      <p className="muted">
        Hermes runs as a local Python service. Setup pulls ~120 packages plus its own
        toolchain — a few minutes.
      </p>
      <button
        type="button"
        onClick={install}
        disabled={starting || running}
        className="rtsetup__btn"
      >
        {running ? phaseLabel(status) : starting ? "Starting…" : "Install Hermes"}
      </button>
      {status?.state === "failed" && <p className="agents__err">{status.error}</p>}
    </div>
  );
}
