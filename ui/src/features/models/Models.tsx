import { useEffect, useMemo, useRef, useState } from "react";
import { SectionNav, type NavSection } from "../../components/SectionNav";
import {
  useAbout,
  useBenchmarks,
  useFeaturedModels,
  useJobs,
  useModels,
  useModelStacks,
  useModelTags,
  usePinnedModels,
} from "../../lib/hooks";
import {
  benchmarkModel,
  enqueueDownload,
  importModel,
  registryModel,
  setModelRoles,
  setModelTags,
  upgradeCheck,
  type Benchmark,
  type FeaturedModel,
  type FitVerdict,
  type Job,
  type KnownModel,
  type Model,
  type ModelStack,
  type ModelType,
  type RegistryDetails,
} from "../../lib/ipc";
import { ColibriPanel } from "./ColibriPanel";
import { Discover, FileRow } from "./Discover";
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

const MODEL_SECTIONS: NavSection[] = [
  { id: "library", label: "Library" },
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
  const [section, setSection] = useState<string>(MODEL_SECTIONS[0].id);
  const importRef = useRef<HTMLDivElement>(null);
  const [importFlash, setImportFlash] = useState(false);

  // "Set import type" (on a catalog/Discover row) changes a dropdown on the
  // Import form, which lives on the "Add models" page -- jump there, then
  // scroll it into view and flash it once that page has actually rendered
  // (a plain scrollIntoView here would run before the section switch
  // commits, when the ref is still null).
  useEffect(() => {
    if (importFlash) importRef.current?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [importFlash]);

  const useType = (t: ModelType) => {
    setModelType(t);
    setSection("add");
    setImportFlash(true);
    setTimeout(() => setImportFlash(false), 1200);
  };

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

          {section === "add" && (
            <>
              <Catalog onUseType={useType} />
              <div ref={importRef} className={importFlash ? "models__flash" : undefined}>
                <ImportForm modelType={modelType} setModelType={setModelType} onImported={refetch} />
              </div>
            </>
          )}

          {section === "discover" && <Discover onUseType={useType} />}

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
        .filter((j: Job) => j.job_type === type && ACTIVE_JOB.has(j.state))
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
          <span>Path to a {typeInfo.ext} file, or a download link</span>
          <input
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

        {!isLink && (
          <label className="chip">
            <input
              type="checkbox"
              checked={keepOriginal}
              onChange={(e) => setKeepOriginal(e.target.checked)}
            />
            keep the original file (copy instead of move)
          </label>
        )}

        <button type="submit" disabled={busy || !trimmed}>
          {busy ? (isLink ? "Queuing…" : "Importing…") : isLink ? "Download & import" : "Import"}
        </button>
      </form>
      {message && <p className={message.kind === "ok" ? "import__ok" : "import__err"}>{message.text}</p>}
    </section>
  );
}

const FIT_COLOR: Record<FitVerdict["level"], string> = {
  green: "var(--load-ok)",
  yellow: "var(--load-warn)",
  red: "var(--load-crit)",
  unknown: "var(--border)",
};

const FIT_LABEL: Record<FitVerdict["level"], string> = {
  green: "Fits comfortably",
  yellow: "Tight fit",
  red: "Won't fit well",
  unknown: "Fit unknown",
};

const fitTitle = (fit: FitVerdict): string =>
  fit.level === "yellow" || fit.level === "red" ? fit.reason : FIT_LABEL[fit.level];

/** A colored dot + plain-language label for a `FitVerdict` — shared by the
 *  Image/Video/Chat/Code catalog rows below. */
function FitBadge({ fit }: { fit: FitVerdict }) {
  return (
    <span className="known__fit" title={fitTitle(fit)}>
      <span className="known__fitdot" style={{ background: FIT_COLOR[fit.level] }} />
      {FIT_LABEL[fit.level]}
    </span>
  );
}

// Values match `KnownModel.media` / `FeaturedModel.role` exactly, so the
// filters below are a plain equality check.
type CatalogTab = "image" | "video" | "chat" | "coding";
const CATALOG_TABS: { value: CatalogTab; label: string; blurb: string }[] = [
  {
    value: "image",
    label: "Image",
    blurb: "A base checkpoint or diffusion model for the Image tab, plus the encoders/VAE it needs.",
  },
  {
    value: "video",
    label: "Video",
    blurb: "A base model for the Video tab, plus its text encoder and VAE.",
  },
  {
    value: "chat",
    label: "Chat",
    blurb: "A general assistant for the Chat tab.",
  },
  {
    value: "coding",
    label: "Code",
    blurb: "Powers an agent session (OpenCode/Hermes) via the coding role.",
  },
];

/** "What should I install, and for what?" — the curated image/video catalogue
 *  (6.x) plus the chat/coding recommendations, grouped into tabs, each
 *  fit-checked against the current VRAM budget and with one pick per group
 *  flagged "★ recommended for your hardware". */
function Catalog({ onUseType }: { onUseType: (t: ModelType) => void }) {
  const stacks = useModelStacks();
  const featured = useFeaturedModels();
  const about = useAbout();
  const [tab, setTab] = useState<CatalogTab>("image");

  const active = CATALOG_TABS.find((t) => t.value === tab)!;
  const stackRows = stacks?.filter((s) => s.media === tab);
  const featuredRows = featured?.filter((m) => m.role === tab);
  const loading = tab === "image" || tab === "video" ? !stacks : !featured;

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Recommended models</h2>
        <span className="card__sub">
          {about
            ? `fit-checked against your ~${(about.vram_budget_mb / 1024).toFixed(0)} GB VRAM budget`
            : "what to install, and for what"}
        </span>
      </header>

      <div className="catalog__tabs" role="tablist">
        {CATALOG_TABS.map((t) => (
          <button
            key={t.value}
            type="button"
            role="tab"
            aria-selected={tab === t.value}
            className={`chip ${tab === t.value ? "chip--on" : ""}`}
            onClick={() => setTab(t.value)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <p className="muted">{active.blurb}</p>

      {loading && <p className="muted">Loading…</p>}
      {!loading && (tab === "image" || tab === "video") && (stackRows?.length ?? 0) === 0 && (
        <p className="muted">Nothing curated here yet.</p>
      )}
      {(tab === "image" || tab === "video") && stackRows && stackRows.length > 0 && (
        <div className="stacklist">
          {stackRows.map((s) => (
            <StackCard key={s.id} stack={s} onUseType={onUseType} />
          ))}
        </div>
      )}
      {!loading && (tab === "chat" || tab === "coding") && (featuredRows?.length ?? 0) === 0 && (
        <p className="muted">Nothing curated here yet.</p>
      )}
      {(tab === "chat" || tab === "coding") && featuredRows && featuredRows.length > 0 && (
        <ul className="known">
          {featuredRows.map((m) => (
            <FeaturedRow key={m.id} model={m} onUseType={onUseType} />
          ))}
        </ul>
      )}
    </section>
  );
}

/** One catalogue file. The URL/SHA-256/size are already pinned (unlike a
 *  Featured pick), so "Download & import" needs no registry lookup — it
 *  queues straight away. `compact` drops the note/file-details line, for use
 *  inside a `StackCard`'s already-labelled member list. */
function KnownRow({
  model,
  onUseType,
  compact,
}: {
  model: KnownModel;
  onUseType: (t: ModelType) => void;
  compact?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const [dl, setDl] = useState<"idle" | "queued" | "error">("idle");

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(model.url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard blocked — the link is still visible below */
    }
  };

  const download = async () => {
    setDl("idle");
    try {
      await enqueueDownload({
        url: model.url,
        filename: model.file,
        model_type: model.kind,
        sha256: model.sha256,
        size_bytes: model.size_bytes,
      });
      setDl("queued");
    } catch {
      setDl("error");
    }
  };

  return (
    <li className="known__row">
      <div className="known__main">
        <div className="known__name">
          {model.name}
          {model.is_default && <span className="badge badge--pick">★ recommended</span>}
        </div>
        <span className="known__badges">
          <span className="badge">{model.kind.replace("_", " ")}</span>
          {model.family && <span className="badge">{model.family}</span>}
          <FitBadge fit={model.fit} />
        </span>
        {!compact && <span className="known__note">{model.note}</span>}
        <span className="known__file numeric">
          {model.file} · {gbBytes(model.size_bytes)} · {model.license}
        </span>
      </div>
      <div className="known__actions">
        <button type="button" onClick={download} disabled={dl === "queued"}>
          {dl === "queued" ? "Queued ✓" : dl === "error" ? "Failed — retry" : "Download & import"}
        </button>
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

/** A base image/video model plus every companion file it needs (VAE, text
 *  encoder, …) — "Download entire stack" queues all of them in one go, so
 *  you don't have to know Flux needs four separate files or hunt them down
 *  one at a time. Every file is still individually downloadable below, for
 *  topping up just the one piece you're missing. */
function StackCard({ stack, onUseType }: { stack: ModelStack; onUseType: (t: ModelType) => void }) {
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<"idle" | "queued" | "error">("idle");

  const totalBytes = stack.members.reduce((sum, m) => sum + m.size_bytes, 0);
  const fit = stack.fit;

  const downloadAll = async () => {
    setBusy(true);
    setStatus("idle");
    try {
      for (const m of stack.members) {
        await enqueueDownload({
          url: m.url,
          filename: m.file,
          model_type: m.kind,
          sha256: m.sha256,
          size_bytes: m.size_bytes,
        });
      }
      setStatus("queued");
    } catch {
      setStatus("error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="stackcard">
      <header className="stackcard__head">
        <div className="known__name">
          {stack.label}
          {stack.is_default && <span className="badge badge--pick">★ recommended</span>}
        </div>
        <span className="known__badges">
          <span className="badge">
            {stack.members.length} file{stack.members.length > 1 ? "s" : ""}
          </span>
          <span className="badge numeric">{gbBytes(totalBytes)} total</span>
          <FitBadge fit={fit} />
        </span>
        <span className="known__note">{stack.note}</span>
      </header>

      <ul className="known stackcard__members">
        {stack.members.map((m) => (
          <KnownRow key={m.id} model={m} onUseType={onUseType} compact />
        ))}
      </ul>

      <div className="stackcard__actions">
        <button type="button" onClick={downloadAll} disabled={busy || status === "queued"}>
          {status === "queued"
            ? `Queued all ${stack.members.length} ✓`
            : status === "error"
              ? "Some failed to queue — check below"
              : busy
                ? "Queuing…"
                : `Download entire stack (${stack.members.length} file${stack.members.length > 1 ? "s" : ""})`}
        </button>
      </div>
    </section>
  );
}

/** A curated chat/coding pick — only a repo + preferred quant is pinned (see
 *  `core::model::FeaturedModel`), so "Show download options" resolves the
 *  real file list live (the same way Discover does) and lists **every**
 *  weight file Hugging Face offers, not just the recommended one — pick a
 *  smaller/bigger quant if you want. Each one-click download carries
 *  `model.import_roles`, so a coding pick actually gets the `coding` role. */
function FeaturedRow({ model, onUseType }: { model: FeaturedModel; onUseType: (t: ModelType) => void }) {
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState<RegistryDetails | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (next && !details && !loading) {
      setLoading(true);
      setErr(null);
      try {
        setDetails(await registryModel(model.repo));
      } catch (e) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    }
  };

  const hint = model.quant_hint.toUpperCase();
  const weightFiles = details?.files.filter((f) => f.quant || f.vram_estimate_mb != null) ?? [];
  const gated = details ? details.gated !== "no" : false;

  const copyRepoLink = async () => {
    try {
      await navigator.clipboard.writeText(`https://huggingface.co/${model.repo}`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard blocked — the link is still visible below */
    }
  };

  return (
    <li className="known__row">
      <div className="known__main">
        <div className="known__name">
          {model.label}
          {model.is_default && <span className="badge badge--pick">★ recommended</span>}
        </div>
        <span className="known__badges">
          <span className="badge">{model.role}</span>
          <FitBadge fit={model.fit} />
        </span>
        <span className="known__note">{model.note}</span>
        <span className="known__file numeric">
          {model.quant_hint} · ~{(model.typical_vram_mb / 1024).toFixed(1)} GB VRAM (estimate) ·{" "}
          {model.license}
        </span>
        <span className="known__note">
          Imports with role{model.import_roles.length > 1 ? "s" : ""}:{" "}
          <code>{model.import_roles.join(", ")}</code>
        </span>

        {open && (
          <div className="discover__files">
            {loading && <p className="muted">Looking up the real file list…</p>}
            {err && <p className="import__err">{err}</p>}
            {details && weightFiles.length === 0 && (
              <p className="muted">No weight files found right now — open the repo on Hugging Face.</p>
            )}
            {weightFiles.map((f) => (
              <FileRow
                key={f.path}
                file={f}
                gated={gated}
                modelType="chat"
                roles={model.import_roles}
                recommended={f.quant?.toUpperCase().includes(hint) ?? f.path.toUpperCase().includes(hint)}
              />
            ))}
          </div>
        )}
      </div>
      <div className="known__actions">
        <button type="button" onClick={toggle}>
          {open ? "Hide" : "Show download options"}
        </button>
        <button type="button" onClick={() => onUseType("chat")}>
          Set import type
        </button>
        <button type="button" onClick={copyRepoLink}>
          {copied ? "Copied ✓" : "Copy repo link"}
        </button>
      </div>
    </li>
  );
}
