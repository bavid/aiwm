import { useMemo, useState } from "react";
import {
  useBenchmarks,
  useJobs,
  useKnownModels,
  useModels,
  useModelTags,
} from "../../lib/hooks";
import {
  benchmarkModel,
  importModel,
  setModelTags,
  upgradeCheck,
  type Benchmark,
  type Job,
  type KnownModel,
  type Model,
  type ModelType,
} from "../../lib/ipc";
import { Discover } from "./Discover";
import { Downloads } from "./Downloads";
import { DeleteButton, StoragePanel } from "./StoragePanel";
import { UpgradeChecks } from "./UpgradeChecks";
import "./models.css";

const ROLES = ["chat", "coding", "reasoning", "embedding"];

const MODEL_TYPES: { value: ModelType; label: string; ext: string }[] = [
  { value: "chat", label: "Chat / LLM", ext: ".gguf" },
  { value: "checkpoint", label: "Image checkpoint", ext: ".safetensors" },
  { value: "diffusion_model", label: "Diffusion model / UNet", ext: ".safetensors, .gguf" },
  { value: "vae", label: "VAE", ext: ".safetensors" },
  { value: "lora", label: "LoRA", ext: ".safetensors" },
  { value: "text_encoder", label: "Text encoder / CLIP", ext: ".safetensors, .gguf" },
  { value: "video", label: "Video model", ext: ".safetensors, .gguf" },
];

const gb = (mb: number | null) => (mb == null ? "—" : `${(mb / 1024).toFixed(1)} GB`);
const gbBytes = (b: number) => `${(b / 1024 ** 3).toFixed(2)} GB`;
const params = (n: number | null) =>
  n == null ? "—" : n >= 1e9 ? `${(n / 1e9).toFixed(1)} B` : `${(n / 1e6).toFixed(0)} M`;
const ctx = (n: number | null) => (n == null ? "—" : n >= 1024 ? `${Math.round(n / 1024)}K` : `${n}`);

/** Not-yet-finished job states — a `bench` job in one of these means "testing". */
const ACTIVE_JOB = new Set(["queued", "scheduled", "blocked", "preparing", "running", "post"]);

const scoreBand = (score: number) =>
  score >= 70 ? "ok" : score >= 40 ? "warn" : "crit";

function scoreTitle(b: Benchmark): string {
  const bits = [
    b.gen_tps != null && `${b.gen_tps.toFixed(1)} tok/s generation`,
    b.prompt_tps != null && `${b.prompt_tps.toFixed(0)} tok/s prompt`,
    b.load_ms != null && `${(b.load_ms / 1000).toFixed(1)} s load`,
    b.vram_peak_mb != null && `${(b.vram_peak_mb / 1024).toFixed(1)} GB VRAM peak`,
    `stability ${(b.stability_score * 100).toFixed(0)}%`,
  ].filter(Boolean);
  return `Heuristic score (speed + fit + stability — not a quality score)\n${bits.join(" · ")}`;
}

export function Models() {
  const { data: models, error, refetch } = useModels();
  const [modelType, setModelType] = useState<ModelType>("chat");

  return (
    <div className="models">
      <ImportForm modelType={modelType} setModelType={setModelType} onImported={refetch} />

      <ModelLibrary models={models} error={error} />

      <Downloads />

      <UpgradeChecks />

      <StoragePanel />

      <Discover onUseType={setModelType} />

      <KnownModels onUseType={setModelType} />
    </div>
  );
}

function ModelLibrary({ models, error }: { models: Model[] | null; error: string | null }) {
  const { data: benchmarks } = useBenchmarks();
  const { data: jobs } = useJobs();
  const { data: tagMap } = useModelTags();
  const [tagFilter, setTagFilter] = useState<string | null>(null);

  const tags = tagMap ?? {};
  const allTags = useMemo(
    () => [...new Set(Object.values(tags).flat())].sort(),
    [tags],
  );

  const byModel = new Map((benchmarks ?? []).map((b) => [b.model_id, b]));
  const targetOf = (j: Job): string => {
    const p = j.params as { target_model_id?: string } | null;
    return p?.target_model_id ?? j.model_id ?? "";
  };
  const activeOf = (type: string) =>
    new Set(
      (jobs ?? [])
        .filter((j: Job) => j.job_type === type && ACTIVE_JOB.has(j.state))
        .map(targetOf),
    );
  const testing = activeOf("bench");
  const checking = activeOf("upgrade_check");

  const shown = (models ?? []).filter(
    (m) => !tagFilter || (tags[m.id] ?? []).includes(tagFilter),
  );

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Model Library</h2>
        <span className="card__sub numeric">
          {tagFilter ? `${shown.length} of ${models?.length ?? 0}` : `${models?.length ?? 0} installed`}
        </span>
      </header>
      {error && <p className="muted">Could not load models: {error}</p>}
      {models && models.length === 0 && <p className="muted">No models yet — import a .gguf above.</p>}
      {allTags.length > 0 && (
        <div className="tagfilter">
          <button
            type="button"
            className={`chip ${tagFilter === null ? "chip--on" : ""}`}
            onClick={() => setTagFilter(null)}
          >
            All
          </button>
          {allTags.map((t) => (
            <button
              key={t}
              type="button"
              className={`chip ${tagFilter === t ? "chip--on" : ""}`}
              onClick={() => setTagFilter(tagFilter === t ? null : t)}
            >
              {t}
            </button>
          ))}
        </div>
      )}
      {models && models.length > 0 && (
        <div className="model-table__scroll">
          <table className="model-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Family</th>
                <th>Quant</th>
                <th>Params</th>
                <th>Size</th>
                <th>Ctx</th>
                <th>VRAM est.</th>
                <th>Score</th>
                <th>Tags</th>
                <th>Roles</th>
                <th>Runtimes</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {shown.map((m: Model) => (
                <tr key={m.id}>
                  <td title={m.file_path}>{m.name}</td>
                  <td className="muted">{m.family ?? m.arch ?? "—"}</td>
                  <td>{m.quant ?? "—"}</td>
                  <td className="numeric">{params(m.param_count)}</td>
                  <td className="numeric">{gb(m.size_bytes / (1024 * 1024))}</td>
                  <td className="numeric">{ctx(m.ctx_max)}</td>
                  <td
                    className="numeric"
                    title="Estimate at load — weights + KV cache / activations + runtime overhead"
                  >
                    {gb(m.vram_estimate_mb)}
                  </td>
                  <td>
                    <ScoreCell
                      model={m}
                      bench={byModel.get(m.id)}
                      testing={testing.has(m.id)}
                    />
                  </td>
                  <td>
                    <TagCell modelId={m.id} tags={tags[m.id] ?? []} />
                  </td>
                  <td className="muted">{m.roles.join(", ") || "—"}</td>
                  <td className="muted">{m.runtimes.join(", ") || "—"}</td>
                  <td className="model-table__actions">
                    <UpgradeCell model={m} checking={checking.has(m.id)} />
                    <DeleteButton model={m} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function ScoreCell({
  model,
  bench,
  testing,
}: {
  model: Model;
  bench: Benchmark | undefined;
  testing: boolean;
}) {
  const [err, setErr] = useState(false);
  const canTest = model.format === "gguf";

  const run = async () => {
    setErr(false);
    try {
      await benchmarkModel(model.id);
    } catch {
      setErr(true);
    }
  };

  if (testing) return <span className="score__testing">testing…</span>;

  return (
    <span className="score">
      {bench && (
        <span className={`score__pill score__pill--${scoreBand(bench.overall_score)}`} title={scoreTitle(bench)}>
          {bench.overall_score}
          {bench.gen_tps != null && <em> · {bench.gen_tps.toFixed(0)} t/s</em>}
        </span>
      )}
      {canTest && (
        <button type="button" className="score__test" onClick={run}>
          {err ? "retry" : bench ? "re-test" : "Test"}
        </button>
      )}
    </span>
  );
}

/** Chips with an ×, plus a "+" that reveals a one-tag input. Optimistic. */
function TagCell({ modelId, tags }: { modelId: string; tags: string[] }) {
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const [local, setLocal] = useState<string[] | null>(null);
  const shown = local ?? tags;

  const save = async (next: string[]) => {
    setLocal(next);
    try {
      const clean = await setModelTags(modelId, next);
      setLocal(clean);
    } catch {
      setLocal(null); // let the poll restore the truth
    }
  };

  const add = () => {
    const t = draft.trim().toLowerCase();
    setDraft("");
    setAdding(false);
    if (t && !shown.includes(t)) void save([...shown, t]);
  };

  return (
    <span className="tagcell">
      {shown.map((t) => (
        <span key={t} className="tagcell__chip">
          {t}
          <button type="button" onClick={() => void save(shown.filter((x) => x !== t))}>
            ×
          </button>
        </span>
      ))}
      {adding ? (
        <input
          autoFocus
          className="tagcell__input"
          value={draft}
          placeholder="tag…"
          spellCheck={false}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") add();
            if (e.key === "Escape") {
              setAdding(false);
              setDraft("");
            }
          }}
          onBlur={add}
        />
      ) : (
        <button type="button" className="tagcell__plus" onClick={() => setAdding(true)}>
          + tag
        </button>
      )}
    </span>
  );
}

/** "Is there something better?" — one online request, so ask first. */
function UpgradeCell({ model, checking }: { model: Model; checking: boolean }) {
  const [state, setState] = useState<"idle" | "error">("idle");

  const run = async () => {
    if (
      !window.confirm(
        `Ask Hugging Face for a newer/bigger model like “${model.name}”, then have your local model rank the results?\n\nThis makes one online request.`,
      )
    )
      return;
    setState("idle");
    try {
      await upgradeCheck(model.id);
    } catch {
      setState("error");
    }
  };

  if (checking) return <span className="score__testing">checking…</span>;
  return (
    <button type="button" className="score__test" onClick={run}>
      {state === "error" ? "retry" : "Better?"}
    </button>
  );
}

function ImportForm({
  modelType,
  setModelType,
  onImported,
}: {
  modelType: ModelType;
  setModelType: (t: ModelType) => void;
  onImported: () => void;
}) {
  const [path, setPath] = useState("");
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

function KnownModels({ onUseType }: { onUseType: (t: ModelType) => void }) {
  const known = useKnownModels();
  if (!known || known.length === 0) return null;

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Known models</h2>
        <span className="card__sub">download from Hugging Face, then import above</span>
      </header>
      <p className="muted">
        A Flux job needs all four pieces: the diffusion GGUF, a T5 encoder, CLIP-L, and the VAE.
        The importer recognizes these by their SHA-256 and fills in the metadata.
      </p>
      <ul className="known">
        {known.map((m) => (
          <KnownRow key={m.id} model={m} onUseType={onUseType} />
        ))}
      </ul>
    </section>
  );
}

function KnownRow({ model, onUseType }: { model: KnownModel; onUseType: (t: ModelType) => void }) {
  const [copied, setCopied] = useState(false);
  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(model.url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard blocked — the link is still visible below */
    }
  };

  return (
    <li className="known__row">
      <div className="known__main">
        <span className="known__name">{model.name}</span>
        <span className="known__badges">
          <span className="badge">{model.kind.replace("_", " ")}</span>
          {model.family && <span className="badge">{model.family}</span>}
        </span>
        <span className="known__note">{model.note}</span>
        <span className="known__file numeric">
          {model.file} · {gbBytes(model.size_bytes)} · {model.license}
        </span>
      </div>
      <div className="known__actions">
        <button type="button" onClick={() => onUseType(model.kind)}>
          Set import type
        </button>
        <button type="button" onClick={copyLink}>
          {copied ? "Copied ✓" : "Copy link"}
        </button>
      </div>
    </li>
  );
}
