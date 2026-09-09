import { useState } from "react";
import { useModels } from "../../lib/hooks";
import { importModel, type Model, type ModelType } from "../../lib/ipc";
import "./models.css";

const ROLES = ["chat", "coding", "reasoning", "embedding"];

const MODEL_TYPES: { value: ModelType; label: string; ext: string }[] = [
  { value: "chat", label: "Chat / LLM", ext: ".gguf" },
  { value: "checkpoint", label: "Image checkpoint", ext: ".safetensors" },
  { value: "diffusion_model", label: "Diffusion model / UNet", ext: ".safetensors, .gguf" },
  { value: "vae", label: "VAE", ext: ".safetensors" },
  { value: "lora", label: "LoRA", ext: ".safetensors" },
  { value: "text_encoder", label: "Text encoder / CLIP", ext: ".safetensors, .gguf" },
];

const gb = (mb: number | null) => (mb == null ? "—" : `${(mb / 1024).toFixed(1)} GB`);
const params = (n: number | null) =>
  n == null ? "—" : n >= 1e9 ? `${(n / 1e9).toFixed(1)} B` : `${(n / 1e6).toFixed(0)} M`;
const ctx = (n: number | null) => (n == null ? "—" : n >= 1024 ? `${Math.round(n / 1024)}K` : `${n}`);

export function Models() {
  const { data: models, error, refetch } = useModels();

  return (
    <div className="models">
      <ImportForm onImported={refetch} />

      <section className="card card--wide">
        <header className="card__head">
          <h2>Model Library</h2>
          <span className="card__sub numeric">{models?.length ?? 0} installed</span>
        </header>
        {error && <p className="muted">Could not load models: {error}</p>}
        {models && models.length === 0 && <p className="muted">No models yet — import a .gguf above.</p>}
        {models && models.length > 0 && (
          <table className="model-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Arch</th>
                <th>Quant</th>
                <th>Params</th>
                <th>Size</th>
                <th>Ctx</th>
                <th>VRAM est.</th>
                <th>Roles</th>
                <th>Runtimes</th>
              </tr>
            </thead>
            <tbody>
              {models.map((m: Model) => (
                <tr key={m.id}>
                  <td title={m.file_path}>{m.name}</td>
                  <td className="muted">{m.arch ?? "—"}</td>
                  <td>{m.quant ?? "—"}</td>
                  <td className="numeric">{params(m.param_count)}</td>
                  <td className="numeric">{gb(m.size_bytes / (1024 * 1024))}</td>
                  <td className="numeric">{ctx(m.ctx_max)}</td>
                  <td
                    className="numeric"
                    title="Estimate at the default chat context — weights + KV cache + runtime overhead"
                  >
                    {gb(m.vram_estimate_mb)}
                  </td>
                  <td className="muted">{m.roles.join(", ") || "—"}</td>
                  <td className="muted">{m.runtimes.join(", ") || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}

function ImportForm({ onImported }: { onImported: () => void }) {
  const [path, setPath] = useState("");
  const [modelType, setModelType] = useState<ModelType>("chat");
  const [roles, setRoles] = useState<string[]>(["chat"]);
  const [keepOriginal, setKeepOriginal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ kind: "ok" | "err"; text: string } | null>(null);

  const isChat = modelType === "chat";
  const typeInfo = MODEL_TYPES.find((t) => t.value === modelType)!;

  const toggle = (role: string) =>
    setRoles((rs) => (rs.includes(role) ? rs.filter((r) => r !== role) : [...rs, role]));

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!path.trim() || busy) return;
    setBusy(true);
    setMessage(null);
    try {
      const out = await importModel(path.trim(), isChat ? roles : [], keepOriginal, modelType);
      setMessage({
        kind: "ok",
        text: out.already_present
          ? `Already imported as “${out.model.name}”.`
          : `Imported “${out.model.name}” — ${out.model.runtimes.join(", ") || "no runtime"}.`,
      });
      setPath("");
      onImported();
    } catch (err) {
      setMessage({ kind: "err", text: err instanceof Error ? err.message : String(err) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Import a model</h2>
      </header>
      <form className="import" onSubmit={submit}>
        <label className="import__field">
          <span>Type</span>
          <select value={modelType} onChange={(e) => setModelType(e.target.value as ModelType)}>
            {MODEL_TYPES.map((t) => (
              <option key={t.value} value={t.value}>
                {t.label} ({t.ext})
              </option>
            ))}
          </select>
        </label>

        <label className="import__field">
          <span>Path to a {typeInfo.ext} file</span>
          <input
            type="text"
            value={path}
            placeholder={
              isChat
                ? "E:\\downloads\\qwen2.5-coder-14b.Q4_K_M.gguf"
                : "E:\\downloads\\sd_xl_base_1.0.safetensors"
            }
            onChange={(e) => setPath(e.target.value)}
            spellCheck={false}
          />
        </label>

        {isChat && (
          <div className="import__roles">
            {ROLES.map((r) => (
              <label key={r} className="chip">
                <input type="checkbox" checked={roles.includes(r)} onChange={() => toggle(r)} />
                {r}
              </label>
            ))}
          </div>
        )}

        <label className="chip">
          <input
            type="checkbox"
            checked={keepOriginal}
            onChange={(e) => setKeepOriginal(e.target.checked)}
          />
          keep the original file (copy instead of move)
        </label>

        <button type="submit" disabled={busy || !path.trim()}>
          {busy ? "Importing…" : "Import"}
        </button>
      </form>
      {message && <p className={message.kind === "ok" ? "import__ok" : "import__err"}>{message.text}</p>}
    </section>
  );
}
