import { useEffect, useId, useMemo, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { SectionNav, type NavSection } from "../../components/SectionNav";
import {
  useBenchmarks,
  useJobs,
  useModels,
  useModelTags,
  usePinnedModels,
} from "../../lib/hooks";
import {
  benchmarkModel,
  enqueueDownload,
  importModel,
  isBenchmarkable,
  isJobActive,
  renameModel,
  setModelRoles,
  setModelTags,
  upgradeCheck,
  type Benchmark,
  type Job,
  type Model,
  type ModelType,
} from "../../lib/ipc";
import { formatGB, formatGiB } from "../../lib/units";
import { Catalog, type CatalogTab } from "./Catalog";
import { ColibriPanel } from "./ColibriPanel";
import { Discover } from "./Discover";
import { Downloads } from "./Downloads";
import { Packages } from "./Packages";
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
  { value: "clip_vision", label: "CLIP vision (character consistency)", ext: ".safetensors" },
  { value: "ip_adapter", label: "IP-Adapter (character consistency)", ext: ".safetensors" },
  { value: "video", label: "Video model", ext: ".safetensors, .gguf" },
  { value: "voice_model", label: "Voice model (TTS)", ext: ".onnx" },
  { value: "voice_data", label: "Voice data (voice presets)", ext: ".bin" },
  { value: "wd_tagger", label: "WD tagger (dataset captioner)", ext: ".onnx, .csv" },
  {
    value: "florence2_engine",
    label: "Florence-2 file (dataset captioner)",
    ext: "one file of the pinned snapshot",
  },
  {
    value: "qwen_vl_engine",
    label: "Qwen2.5-VL file (caption second opinion)",
    ext: "one file of the pinned snapshot",
  },
];

/** A request from another tab to open a specific part of this one — today
 *  only the Dataset tab's "More captioners…" link. */
export type ModelsFocus = "training";

type ModelsProps = {
  focus: ModelsFocus | null;
  /** Clears the hand-over so re-visiting the tab does not jump again. */
  onFocusConsumed: () => void;
};

const MODEL_SECTIONS: NavSection[] = [
  { id: "library", label: "Library" },
  { id: "packages", label: "Packages" },
  { id: "add", label: "Add models" },
  { id: "discover", label: "Discover" },
  { id: "downloads", label: "Downloads" },
  { id: "maintenance", label: "Maintenance" },
];

/** The last path segment of a URL, query/fragment stripped -- a reasonable
 *  download filename when the user pastes a plain link instead of a path. */
function filenameFromUrl(url: string): string {
  try {
    const { pathname } = new URL(url);
    const last = pathname.split("/").filter(Boolean).pop();
    return last ? decodeURIComponent(last) : "download";
  } catch {
    return "download";
  }
}

const params = (n: number | null) =>
  n == null ? "—" : n >= 1e9 ? `${(n / 1e9).toFixed(1)} B` : `${(n / 1e6).toFixed(0)} M`;
const ctx = (n: number | null) => (n == null ? "—" : n >= 1024 ? `${Math.round(n / 1024)}K` : `${n}`);

const scoreBand = (score: number) =>
  score >= 70 ? "ok" : score >= 40 ? "warn" : "crit";

function scoreTitle(b: Benchmark): string {
  const bits = [
    b.gen_tps != null && `${b.gen_tps.toFixed(1)} tok/s generation`,
    b.prompt_tps != null && `${b.prompt_tps.toFixed(0)} tok/s prompt`,
    b.load_ms != null && `${(b.load_ms / 1000).toFixed(1)} s load`,
    b.vram_peak_mb != null && `${formatGiB(b.vram_peak_mb)} VRAM peak`,
    `stability ${(b.stability_score * 100).toFixed(0)}%`,
  ].filter(Boolean);
  return `Heuristic score (speed + fit + stability — not a quality score)\n${bits.join(" · ")}`;
}

export function Models({ focus, onFocusConsumed }: ModelsProps) {
  const { data: models, error, refetch } = useModels();
  const [modelType, setModelType] = useState<ModelType>("chat");
  const [section, setSection] = useState<string>(MODEL_SECTIONS[0].id);
  const [catalogTab, setCatalogTab] = useState<CatalogTab>("image");
  /** Bumped to make the catalog scroll into view and focus its active tab. */
  const [catalogFocusRequest, setCatalogFocusRequest] = useState(0);

  useEffect(() => {
    if (focus !== "training") return;
    setSection("add");
    setCatalogTab("training");
    setCatalogFocusRequest((n) => n + 1);
    onFocusConsumed();
  }, [focus, onFocusConsumed]);

  return (
    <div className="models">
      <header className="models__head">
        <h1>Models</h1>
      </header>
      <div className="models-body">
        <SectionNav
          sections={MODEL_SECTIONS}
          activeId={section}
          onChange={setSection}
          ariaLabel="Models sections"
        />
        <div className="models-sections">
          {section === "library" && <ModelLibrary models={models} error={error} />}

          {section === "packages" && (
            <Packages
              onViewDownloads={() => setSection("downloads")}
              onAddModels={() => setSection("add")}
            />
          )}

          {section === "add" && (
            <>
              <Catalog
                tab={catalogTab}
                onTabChange={setCatalogTab}
                focusRequest={catalogFocusRequest}
              />
              <ImportForm modelType={modelType} setModelType={setModelType} onImported={refetch} />
            </>
          )}

          {section === "discover" && <Discover onViewDownloads={() => setSection("downloads")} />}

          {section === "downloads" && <Downloads />}

          {section === "maintenance" && (
            <>
              <UpgradeChecks />
              <StoragePanel />
              <ColibriPanel />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function ModelLibrary({ models, error }: { models: Model[] | null; error: string | null }) {
  const { data: benchmarks } = useBenchmarks();
  const { data: jobs } = useJobs();
  const { data: tagMap } = useModelTags();
  const { isPinned, toggle: togglePin } = usePinnedModels();
  const [tagFilter, setTagFilter] = useState<string | null>(null);

  const tags = useMemo(() => tagMap ?? {}, [tagMap]);
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
        .filter((j: Job) => j.job_type === type && isJobActive(j.state))
        .map(targetOf),
    );
  const testing = activeOf("bench");
  const checking = activeOf("upgrade_check");

  const shown = (models ?? [])
    .filter((m) => !tagFilter || (tags[m.id] ?? []).includes(tagFilter))
    .slice()
    .sort((a, b) => Number(isPinned(b.id)) - Number(isPinned(a.id)));

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
                <th />
                <th>Name</th>
                <th>Family</th>
                <th>Quant</th>
                <th>Params</th>
                <th>Size</th>
                <th>Ctx</th>
                <th>
                  VRAM est. <HelpHint area="models" setting="vram-estimate" />
                </th>
                <th>
                  Score <HelpHint area="models" setting="score" />
                </th>
                <th>Tags</th>
                <th>
                  Roles <HelpHint area="models" setting="roles" />
                </th>
                <th>Runtimes</th>
                <th>
                  <span className="visually-hidden">Actions</span>{" "}
                  <HelpHint area="models" setting="better" />
                </th>
              </tr>
            </thead>
            <tbody>
              {shown.map((m: Model) => (
                <tr key={m.id}>
                  <td>
                    <button
                      type="button"
                      className="pin-star"
                      data-on={isPinned(m.id)}
                      onClick={() => togglePin(m.id)}
                      title={isPinned(m.id) ? "Unpin" : "Pin for quick access"}
                      aria-label={isPinned(m.id) ? `Unpin ${m.name}` : `Pin ${m.name}`}
                    >
                      {isPinned(m.id) ? "★" : "☆"}
                    </button>
                  </td>
                  <td title={m.file_path}>
                    <NameCell modelId={m.id} name={m.name} />
                  </td>
                  <td className="muted">{m.family ?? m.arch ?? "—"}</td>
                  <td>{m.quant ?? "—"}</td>
                  <td className="numeric">{params(m.param_count)}</td>
                  <td className="numeric">{formatGB(m.size_bytes)}</td>
                  <td className="numeric">{ctx(m.ctx_max)}</td>
                  <td className="numeric">
                    {m.vram_estimate_mb == null ? "—" : formatGiB(m.vram_estimate_mb)}
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
                  <td>
                    <RoleCell modelId={m.id} roles={m.roles} editable={m.format === "gguf"} />
                  </td>
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
  const canTest = isBenchmarkable(model);

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

/** Click the name to edit it in place -- Enter/blur saves, Escape reverts.
 *  Optimistic, same pattern as `TagCell` below. Renaming only ever touches
 *  the library's own `name` column; the file on disk keeps its own name. */
function NameCell({ modelId, name }: { modelId: string; name: string }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(name);
  const [local, setLocal] = useState<string | null>(null);
  const shown = local ?? name;

  const start = () => {
    setDraft(shown);
    setEditing(true);
  };

  const save = async () => {
    setEditing(false);
    const next = draft.trim();
    if (!next || next === shown) return;
    setLocal(next);
    try {
      const m = await renameModel(modelId, next);
      setLocal(m.name);
    } catch {
      setLocal(null); // let the poll restore the truth
    }
  };

  if (editing) {
    return (
      <input
        autoFocus
        className="namecell__input"
        value={draft}
        spellCheck={false}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            setDraft(shown);
            setEditing(false);
          }
        }}
        onBlur={save}
      />
    );
  }

  return (
    <button type="button" className="namecell" onClick={start}>
      {shown}
      <span className="visually-hidden"> — rename</span>
    </button>
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

/** The four functional roles the UI lets you toggle by hand (see `ROLES`
 *  above the import form). `base_diffusion`/`base_video`/`vae`/`text_encoder`
 *  are auto-assigned at import from the file kind and stay out of this list —
 *  toggling them here wouldn't mean anything. */
const EDITABLE_ROLES = ["chat", "coding", "reasoning", "embedding"];

/** Toggle chips for a GGUF model's roles — e.g. add `coding` to a model that
 *  was downloaded before that role existed, without re-importing it.
 *  Non-GGUF models (image/video) show their roles as plain text; those are
 *  auto-managed and not meant to be hand-edited here. */
function RoleCell({
  modelId,
  roles,
  editable,
}: {
  modelId: string;
  roles: string[];
  editable: boolean;
}) {
  const [local, setLocal] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const shown = local ?? roles;

  if (!editable) return <span className="muted">{shown.join(", ") || "—"}</span>;

  const toggle = async (role: string) => {
    const next = shown.includes(role) ? shown.filter((r) => r !== role) : [...shown, role];
    setLocal(next);
    setBusy(true);
    try {
      const clean = await setModelRoles(modelId, next);
      setLocal(clean);
    } catch {
      setLocal(null); // let the poll restore the truth
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="rolecell">
      {EDITABLE_ROLES.map((r) => (
        <button
          key={r}
          type="button"
          className={`chip ${shown.includes(r) ? "chip--on" : ""}`}
          disabled={busy}
          onClick={() => toggle(r)}
        >
          {r}
        </button>
      ))}
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
  const ids = useId();
  const typeId = `${ids}-type`;
  const pathId = `${ids}-path`;
  const keepId = `${ids}-keep`;

  const isChat = modelType === "chat";
  const typeInfo = MODEL_TYPES.find((t) => t.value === modelType)!;
  const trimmed = path.trim();
  const isLink = /^https?:\/\//i.test(trimmed);

  const toggle = (role: string) =>
    setRoles((rs) => (rs.includes(role) ? rs.filter((r) => r !== role) : [...rs, role]));

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!trimmed || busy) return;
    setBusy(true);
    setMessage(null);
    try {
      if (isLink) {
        // A link -> queue it (the download manager verifies + auto-imports
        // once it lands, same as Discover's "Download & import").
        const filename = filenameFromUrl(trimmed);
        await enqueueDownload({
          url: trimmed,
          filename,
          model_type: modelType,
          roles: isChat ? roles : [],
        });
        setMessage({ kind: "ok", text: `Queued “${filename}” — see Downloads below.` });
      } else {
        const out = await importModel(trimmed, isChat ? roles : [], keepOriginal, modelType);
        setMessage({
          kind: "ok",
          text: out.already_present
            ? `Already imported as “${out.model.name}”.`
            : `Imported “${out.model.name}” — ${out.model.runtimes.join(", ") || "no runtime"}.`,
        });
      }
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
        <div className="import__field">
          <span>
            <label htmlFor={typeId}>Type</label>
            <HelpHint area="models" setting="import-type" describes={typeId} />
          </span>
          <select
            id={typeId}
            value={modelType}
            onChange={(e) => setModelType(e.target.value as ModelType)}
          >
            {MODEL_TYPES.map((t) => (
              <option key={t.value} value={t.value}>
                {t.label} ({t.ext})
              </option>
            ))}
          </select>
        </div>

        <div className="import__field">
          <span>
            <label htmlFor={pathId}>Path to a {typeInfo.ext} file, or a download link</label>
            <HelpHint area="models" setting="import-path" describes={pathId} />
          </span>
          <input
            id={pathId}
            type="text"
            value={path}
            placeholder={
              isChat
                ? "E:\\downloads\\qwen2.5-coder-14b.Q4_K_M.gguf, or https://huggingface.co/…/resolve/main/…gguf"
                : "E:\\downloads\\sd_xl_base_1.0.safetensors, or a link"
            }
            onChange={(e) => setPath(e.target.value)}
            spellCheck={false}
          />
        </div>

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

        {!isLink && (
          <span className="import__keep">
            <span className="chip">
              <input
                id={keepId}
                type="checkbox"
                checked={keepOriginal}
                onChange={(e) => setKeepOriginal(e.target.checked)}
              />
              <label htmlFor={keepId}>keep the original file (copy instead of move)</label>
            </span>
            <HelpHint area="models" setting="keep-original" describes={keepId} />
          </span>
        )}

        <button type="submit" disabled={busy || !trimmed}>
          {busy ? (isLink ? "Queuing…" : "Importing…") : isLink ? "Download & import" : "Import"}
        </button>
      </form>
      {message && <p className={message.kind === "ok" ? "import__ok" : "import__err"}>{message.text}</p>}
    </section>
  );
}
