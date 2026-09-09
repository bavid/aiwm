import { useState } from "react";
import { createAgent } from "../../lib/ipc";

/** The collapsible "New profile" form. `codingModels` are the library models
 *  carrying the `coding` role. */
export function NewProfileForm({
  codingModels,
  onCreated,
}: {
  codingModels: { id: string; name: string }[];
  onCreated: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [allowed, setAllowed] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !workspace.trim() || busy) return;
    setBusy(true);
    setErr(null);
    try {
      await createAgent({
        name: name.trim(),
        adapter: "opencode",
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
        <select value="opencode" disabled>
          <option value="opencode">OpenCode</option>
          <option value="hermes">Hermes (Phase 5.4)</option>
        </select>
      </label>

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
          No model has the “coding” role yet — import a coding GGUF (e.g. Qwen2.5-Coder) on the
          Models tab and tick “coding”.
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
        <button type="submit" disabled={busy || !name.trim() || !workspace.trim()}>
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
