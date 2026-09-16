import { useEffect, useMemo, useState } from "react";
import { useCivitaiSearch, useModels, useRegistrySearch } from "../../lib/hooks";
import {
  cancelJob,
  civitaiModel,
  enqueueDownload,
  jobDetail,
  registryModel,
  submitJob,
  type CivitaiSearchParams,
  type FitVerdict,
  type Freshness,
  type JobState,
  type ModelType,
  type RecommendCandidate,
  type RecommendKind,
  type RecommendReport,
  type RegistryDetails,
  type RegistryFile,
  type RegistrySearchParams,
  type RemoteModel,
} from "../../lib/ipc";
import { filterDisplayTags } from "../../lib/tags";

/** Which Discover source is active. Civitai has no "ask my local model"
 *  ranking integration (yet) — that toggle only ever applies to the Hugging
 *  Face source. */
type DiscoverSource = "huggingface" | "civitai";
const SOURCES: { value: DiscoverSource; label: string }[] = [
  { value: "huggingface", label: "Hugging Face" },
  { value: "civitai", label: "Civitai" },
];

const RECENT_KEY = "aiwm.discover.recent";

function loadRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.slice(0, 6).map(String) : [];
  } catch {
    return [];
  }
}

function pushRecent(term: string): string[] {
  const next = [term, ...loadRecent().filter((t) => t !== term)].slice(0, 6);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    /* private window — the list is a convenience */
  }
  return next;
}

/** `RemoteModel.format` → the `import_model` type (a first guess; the user can
 *  change the role on the Models table afterwards). */
const importTypeFor = (format: RemoteModel["format"]): ModelType =>
  format === "gguf" ? "chat" : "checkpoint";

const SORTS: { value: NonNullable<RegistrySearchParams["sort"]>; label: string }[] = [
  { value: "downloads", label: "Most downloaded" },
  { value: "likes", label: "Most liked" },
  { value: "trending", label: "Trending" },
  { value: "updated", label: "Recently updated" },
  { value: "new", label: "Newest" },
];

const RECOMMEND_KINDS: { value: RecommendKind; label: string }[] = [
  { value: "chat", label: "Chat model" },
  { value: "coding", label: "Coding model" },
  { value: "image", label: "Image model" },
  { value: "video", label: "Video model" },
  { value: "lora", label: "LoRA" },
];

const JOB_DONE: JobState[] = ["completed", "failed", "cancelled"];

const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(2)} GB`;
const count = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : `${n}`;
const params = (n: number | null) => (n == null ? "—" : n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : `${(n / 1e6).toFixed(0)}M`);

const FIT_COLOR: Record<FitVerdict["level"], string> = {
  green: "var(--load-ok)",
  yellow: "var(--load-warn)",
  red: "var(--load-crit)",
  unknown: "var(--border)",
};

/** A `.gguf` file can never be a checkpoint/VAE/LoRA (`core::model::kind`'s
 *  own rule -- those only ever accept `.safetensors`). The repo-level format
 *  guess that picked `modelType` upstream (`importTypeFor`/`guessModelType`)
 *  trusts Hugging Face's own tags, and a community repo that never got
 *  tagged `gguf` reports as `other` even though its real files are .gguf --
 *  correct it here from the one thing that's always trustworthy: the actual
 *  file being downloaded. */
function safeModelType(modelType: ModelType, path: string): ModelType {
  const isGguf = path.toLowerCase().endsWith(".gguf");
  const ggufIncompatible: ModelType[] = ["checkpoint", "vae", "lora"];
  return isGguf && ggufIncompatible.includes(modelType) ? "chat" : modelType;
}

function fitTitle(fit: FitVerdict, vramMb: number | null): string {
  const est = vramMb ? ` · ~${(vramMb / 1024).toFixed(1)} GB VRAM` : "";
  if (fit.level === "green") return `Fits comfortably${est}`;
  if (fit.level === "unknown") return "Fit unknown";
  return `${fit.reason}${est}`;
}

function freshnessNote(f: Freshness, sourceLabel = "Hugging Face"): string | null {
  if (f.kind === "live") return null;
  const mins = Math.max(1, Math.round(f.age_secs / 60));
  return f.kind === "offline"
    ? `Offline — showing a cached result from ~${mins} min ago.`
    : `${sourceLabel} was unreachable — showing a cached result from ~${mins} min ago.`;
}

/** The user's chosen kind is a far better signal than the file format alone
 *  (format only tells you gguf vs not) -- the user can still correct it via
 *  "Set import type" / the expanded file list. */
function guessModelType(kind: RecommendKind, format: RecommendCandidate["format"]): ModelType {
  switch (kind) {
    case "chat":
    case "coding":
      return "chat";
    case "video":
      return "video";
    case "lora":
      return "lora";
    case "image":
    default:
      return format === "gguf" ? "diffusion_model" : "checkpoint";
  }
}

function rolesFor(kind: RecommendKind): string[] | undefined {
  if (kind === "chat") return ["chat"];
  if (kind === "coding") return ["chat", "coding"];
  return undefined;
}

/** One search box, two ways to use it: plain browsing (live-as-you-type,
 *  sorted by objective signal — downloads/likes/date) or "ask my local
 *  model" (an explicit search that filters spam and hardware-fit, then has
 *  your own chat/coding model pick a shortlist and explain why). They used
 *  to be two separate pages; splitting "search Hugging Face" into two nav
 *  entries was more surface area than the difference was worth. */
export function Discover({ onUseType }: { onUseType: (t: ModelType) => void }) {
  const [source, setSource] = useState<DiscoverSource>("huggingface");
  const [aiMode, setAiMode] = useState(false);
  const [query, setQuery] = useState("");

  const subtitle =
    source === "civitai"
      ? "search Civitai · download in your browser, then import above"
      : aiMode
        ? "your local model ranks & explains the results"
        : "search Hugging Face · download in your browser, then import above";

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Discover models</h2>
        <span className="card__sub">{subtitle}</span>
      </header>

      <div className="catalog__tabs" role="tablist" aria-label="Discover source">
        {SOURCES.map((s) => (
          <button
            key={s.value}
            type="button"
            role="tab"
            aria-selected={source === s.value}
            className={`chip ${source === s.value ? "chip--on" : ""}`}
            onClick={() => setSource(s.value)}
          >
            {s.label}
          </button>
        ))}
      </div>

      {source === "huggingface" && (
        <>
          <label className="chip discover__aitoggle">
            <input type="checkbox" checked={aiMode} onChange={(e) => setAiMode(e.target.checked)} />
            Ask my local model to rank &amp; explain
          </label>

          {aiMode ? (
            <AiSearch query={query} setQuery={setQuery} />
          ) : (
            <PlainSearch query={query} setQuery={setQuery} onUseType={onUseType} />
          )}
        </>
      )}

      {source === "civitai" && <CivitaiSearch onUseType={onUseType} />}
    </section>
  );
}

/** The original Discover behavior: live-as-you-type, no local-model cost. */
function PlainSearch({
  query,
  setQuery,
  onUseType,
}: {
  query: string;
  setQuery: (q: string) => void;
  onUseType: (t: ModelType) => void;
}) {
  const [gguf, setGguf] = useState(true);
  const [sort, setSort] = useState<NonNullable<RegistrySearchParams["sort"]>>("downloads");

  const trimmed = query.trim();
  const enabled = trimmed.length >= 2;
  const searchParams = useMemo<RegistrySearchParams>(
    () => ({ q: trimmed || undefined, gguf, sort, limit: 20 }),
    [trimmed, gguf, sort],
  );
  const { result, error, loading } = useRegistrySearch(searchParams, enabled);
  const note = result ? freshnessNote(result.freshness) : null;

  const [recent, setRecent] = useState<string[]>(loadRecent);
  useEffect(() => {
    if (enabled && result && result.data.length > 0) {
      setRecent(pushRecent(trimmed));
    }
  }, [enabled, result, trimmed]);

  return (
    <>
      <div className="discover__controls">
        <input
          type="text"
          className="discover__search"
          value={query}
          placeholder="qwen2.5 coder, flux, whisper…"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
        />
        <label className="chip">
          <input type="checkbox" checked={gguf} onChange={(e) => setGguf(e.target.checked)} />
          GGUF only
        </label>
        <select value={sort} onChange={(e) => setSort(e.target.value as typeof sort)}>
          {SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
      </div>

      {!enabled && recent.length > 0 && (
        <div className="discover__recent">
          <span className="muted">recent:</span>
          {recent.map((t) => (
            <button key={t} type="button" className="chip" onClick={() => setQuery(t)}>
              {t}
            </button>
          ))}
        </div>
      )}

      {!enabled && <p className="muted">Type at least two characters to search.</p>}
      {enabled && loading && !result && <p className="muted">Searching…</p>}
      {error && <p className="import__err">{error}</p>}
      {note && <p className="discover__stale">{note}</p>}
      {result && result.data.length === 0 && !loading && (
        <p className="muted">No models match — try a broader term or turn off “GGUF only”.</p>
      )}

      <ul className="discover__results">
        {result?.data.map((m) => (
          <ResultCard key={m.id} model={m} onUseType={onUseType} />
        ))}
      </ul>
    </>
  );
}

/** The local-LLM-assisted search: an explicit submit (it costs a job + a
 *  completion, so no live-as-you-type), fit-filtered and spam-dropped,
 *  ranked and explained by whichever chat/coding model you pick. */
function AiSearch({ query, setQuery }: { query: string; setQuery: (q: string) => void }) {
  const { data: models } = useModels();
  const reasonerModels = (models ?? []).filter(
    (m) => m.roles.includes("chat") || m.roles.includes("coding"),
  );

  const [kind, setKind] = useState<RecommendKind>("chat");
  const [reasonerId, setReasonerId] = useState("auto");
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [report, setReport] = useState<RecommendReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    const tick = async () => {
      let detail;
      try {
        detail = await jobDetail(pendingId);
      } catch {
        return;
      }
      if (!alive || !detail) return;
      const { job } = detail;
      if (!JOB_DONE.includes(job.state)) {
        // `blocked` (not enough VRAM right now) isn't terminal -- the
        // scheduler re-checks every tick and can recover on its own once
        // something else frees VRAM -- so keep polling, just show why it's
        // stuck instead of a silent "Searching..." forever.
        setError(job.state === "blocked" ? (job.error_text ?? "not enough VRAM free right now") : null);
        return;
      }
      setPendingId(null);
      if (job.state === "completed" && job.result) {
        try {
          setReport(JSON.parse(job.result) as RecommendReport);
        } catch {
          setError("could not read the recommendation result");
        }
      } else if (job.state === "failed") {
        setError(job.error_text ?? "the search failed");
      }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId]);

  const search = async () => {
    const text = query.trim();
    if (!text || pendingId) return;
    setError(null);
    setReport(null);
    try {
      const job = await submitJob({
        job_type: "recommend",
        params: {
          query: text,
          kind,
          ...(reasonerId !== "auto" ? { reasoner_model_id: reasonerId } : {}),
        },
      });
      setPendingId(job.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <>
      <form
        className="discover__controls"
        onSubmit={(e) => {
          e.preventDefault();
          search();
        }}
      >
        <input
          type="text"
          className="discover__search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="e.g. realistic uncensored nsfw, or best local coding model"
          spellCheck
        />
        <select value={kind} onChange={(e) => setKind(e.target.value as RecommendKind)}>
          {RECOMMEND_KINDS.map((k) => (
            <option key={k.value} value={k.value}>
              {k.label}
            </option>
          ))}
        </select>
        <select
          value={reasonerId}
          onChange={(e) => setReasonerId(e.target.value)}
          title="Which local model reasons about the results"
        >
          <option value="auto">Auto (most-recently-used)</option>
          {reasonerModels.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
        <button type="submit" disabled={!query.trim() || !!pendingId}>
          {pendingId ? "Searching…" : "Search"}
        </button>
        {pendingId && (
          <button
            type="button"
            className="chip"
            onClick={() => {
              cancelJob(pendingId).catch(() => {});
              setPendingId(null);
              setError(null);
            }}
          >
            Cancel
          </button>
        )}
      </form>

      {error && <p className="import__err">{error}</p>}
      {report && (
        <>
          <p className="muted">{report.note}</p>
          {report.freshness.kind !== "live" && (
            <p className="muted">
              {report.freshness.kind === "offline"
                ? "Offline — cached results."
                : "Hugging Face was unreachable — showing the last cached results."}
            </p>
          )}
          {report.candidates.length === 0 && (
            <p className="muted">Nothing matched — try broader wording.</p>
          )}
          <ul className="discover__results">
            {report.candidates.map((c) => (
              <RecommendCard key={c.id} candidate={c} kind={kind} />
            ))}
          </ul>
        </>
      )}
    </>
  );
}

const CIVITAI_TYPES: { value: string; label: string }[] = [
  { value: "Checkpoint", label: "Checkpoints" },
  { value: "LORA", label: "LoRAs" },
];

const CIVITAI_SORTS: { value: NonNullable<CivitaiSearchParams["sort"]>; label: string }[] = [
  { value: "downloads", label: "Most downloaded" },
  { value: "likes", label: "Highest rated" },
  { value: "trending", label: "Trending" },
  { value: "new", label: "Newest" },
];

/** Civitai's own `type` (`model_kind_hint`) is a far better signal than the
 *  generic `format` guess `importTypeFor` uses for Hugging Face — Civitai
 *  explicitly labels Checkpoint vs LoRA, which `format` alone can't tell
 *  apart. Falls back to `importTypeFor` for a type Civitai has that AIWM
 *  doesn't specifically handle. */
function civitaiModelType(hint: string | null, format: RemoteModel["format"]): ModelType {
  switch (hint) {
    case "LORA":
      return "lora";
    case "Checkpoint":
      return format === "gguf" ? "diffusion_model" : "checkpoint";
    default:
      return importTypeFor(format);
  }
}

/** The Civitai side of Discover: its own filters (a type toggle instead of
 *  "GGUF only", an explicit NSFW opt-in instead of nothing) against its own
 *  search endpoint. No "ask my local model" mode — that ranking job only
 *  understands the Hugging Face registry today. */
function CivitaiSearch({ onUseType }: { onUseType: (t: ModelType) => void }) {
  const [query, setQuery] = useState("");
  const [types, setTypes] = useState<string[]>(["Checkpoint", "LORA"]);
  const [sort, setSort] = useState<NonNullable<CivitaiSearchParams["sort"]>>("downloads");
  // Off by default -- Civitai is an open, anonymous-upload platform where
  // NSFW content is common and explicitly tagged, unlike Hugging Face. The
  // user must tick this themselves to see it.
  const [nsfw, setNsfw] = useState(false);

  const trimmed = query.trim();
  const searchParams = useMemo<CivitaiSearchParams>(
    () => ({ q: trimmed || undefined, types, sort, nsfw, limit: 20 }),
    [trimmed, types, sort, nsfw],
  );
  const { result, error, loading } = useCivitaiSearch(searchParams, true);
  const note = result ? freshnessNote(result.freshness, "Civitai") : null;

  const toggleType = (t: string) =>
    setTypes((prev) => (prev.includes(t) ? prev.filter((x) => x !== t) : [...prev, t]));

  return (
    <>
      <div className="discover__controls">
        <input
          type="text"
          className="discover__search"
          value={query}
          placeholder="pony, realistic, anime style…"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
        />
        {CIVITAI_TYPES.map((t) => (
          <label key={t.value} className="chip">
            <input
              type="checkbox"
              checked={types.includes(t.value)}
              onChange={() => toggleType(t.value)}
            />
            {t.label}
          </label>
        ))}
        <select value={sort} onChange={(e) => setSort(e.target.value as typeof sort)}>
          {CIVITAI_SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        <label
          className="chip"
          title="Off by default — Civitai is an open upload platform where NSFW content is common, unlike Hugging Face"
        >
          <input type="checkbox" checked={nsfw} onChange={(e) => setNsfw(e.target.checked)} />
          Show NSFW
        </label>
      </div>

      {loading && !result && <p className="muted">Searching…</p>}
      {error && <p className="import__err">{error}</p>}
      {note && <p className="discover__stale">{note}</p>}
      {types.length === 0 && (
        <p className="muted">Pick at least one type (Checkpoints / LoRAs) to search.</p>
      )}
      {result && result.data.length === 0 && !loading && types.length > 0 && (
        <p className="muted">No models match — try a broader term or turn on “Show NSFW”.</p>
      )}

      <ul className="discover__results">
        {result?.data.map((m) => (
          <CivitaiResultCard key={m.id} model={m} onUseType={onUseType} />
        ))}
      </ul>
    </>
  );
}

function CivitaiResultCard({
  model,
  onUseType,
}: {
  model: RemoteModel;
  onUseType: (t: ModelType) => void;
}) {
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState<RegistryDetails | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (next && !details && !loading) {
      setLoading(true);
      setErr(null);
      try {
        setDetails(await civitaiModel(model.id));
      } catch (e) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    }
  };

  const modelType = civitaiModelType(model.model_kind_hint, model.format);

  return (
    <li className="discover__row">
      {model.preview_image_url && (
        <img
          src={model.preview_image_url}
          alt=""
          className="discover__preview"
          width={72}
          height={72}
          loading="lazy"
        />
      )}
      <div className="discover__main">
        <div className="discover__title">
          <a
            href={`https://civitai.com/models/${model.id}`}
            target="_blank"
            rel="noopener noreferrer"
          >
            {model.name ?? model.id}
          </a>
          {model.nsfw && <span className="badge badge--warn">NSFW</span>}
          {model.model_kind_hint && <span className="badge">{model.model_kind_hint}</span>}
        </div>
        <div className="discover__meta numeric">
          ↓ {count(model.downloads)} · 👍 {count(model.likes)}
          {model.base_model_family && ` · ${model.base_model_family}`}
        </div>
        {model.allow_commercial_use.length > 0 && (
          <div className="discover__meta">
            commercial use: {model.allow_commercial_use.join(", ")}
          </div>
        )}
        <DiscoverTags tags={model.tags} />

        {open && (
          <div className="discover__files">
            {loading && <p className="muted">Loading files…</p>}
            {err && <p className="import__err">{err}</p>}
            {details?.files.map((f) => (
              <FileRow
                key={f.path}
                file={f}
                gated={false}
                modelType={safeModelType(modelType, f.path)}
              />
            ))}
            {details && details.files.length === 0 && (
              <p className="muted">No files listed for this model.</p>
            )}
          </div>
        )}
      </div>

      <div className="known__actions">
        <button type="button" onClick={toggle}>
          {open ? "Hide files" : "Files"}
        </button>
        <button type="button" onClick={() => onUseType(modelType)}>
          Set import type
        </button>
      </div>
    </li>
  );
}

function ResultCard({ model, onUseType }: { model: RemoteModel; onUseType: (t: ModelType) => void }) {
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState<RegistryDetails | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (next && !details && !loading) {
      setLoading(true);
      setErr(null);
      try {
        setDetails(await registryModel(model.id));
      } catch (e) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    }
  };

  return (
    <li className="discover__row">
      <div className="discover__main">
        <div className="discover__title">
          <a href={`https://huggingface.co/${model.id}`} target="_blank" rel="noopener noreferrer">
            {model.id}
          </a>
          {model.gated !== "no" && <span className="badge badge--warn">gated</span>}
          {model.format !== "other" && <span className="badge">{model.format}</span>}
        </div>
        <div className="discover__meta numeric">
          {params(model.param_count)} params · ↓ {count(model.downloads)} · ♥ {count(model.likes)}
          {model.license && ` · ${model.license}`}
          {model.ctx_max && ` · ${Math.round(model.ctx_max / 1024)}K ctx`}
        </div>
        {model.base_model && (
          <div className="discover__meta">
            quant of <code>{model.base_model}</code>
          </div>
        )}
        <DiscoverTags tags={model.tags} />

        {open && (
          <div className="discover__files">
            {loading && <p className="muted">Loading files…</p>}
            {err && <p className="import__err">{err}</p>}
            {details?.files
              .filter((f) => f.quant || f.vram_estimate_mb != null)
              .map((f) => (
                <FileRow
                  key={f.path}
                  file={f}
                  gated={model.gated !== "no"}
                  modelType={importTypeFor(model.format)}
                />
              ))}
            {details && details.files.every((f) => !f.quant && f.vram_estimate_mb == null) && (
              <p className="muted">No weight files detected in this repo.</p>
            )}
          </div>
        )}
      </div>

      <div className="known__actions">
        <button type="button" onClick={toggle}>
          {open ? "Hide files" : "Files"}
        </button>
        <button type="button" onClick={() => onUseType(importTypeFor(model.format))}>
          Set import type
        </button>
      </div>
    </li>
  );
}

function RecommendCard({ candidate, kind }: { candidate: RecommendCandidate; kind: RecommendKind }) {
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState<RegistryDetails | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (next && !details && !loading) {
      setLoading(true);
      setErr(null);
      try {
        setDetails(await registryModel(candidate.id));
      } catch (e) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    }
  };

  const tags = filterDisplayTags(candidate.tags);

  return (
    <li className="discover__row">
      <div className="discover__main">
        <div className="discover__title">
          <a href={`https://huggingface.co/${candidate.id}`} target="_blank" rel="noopener noreferrer">
            {candidate.id}
          </a>
          {candidate.gated && <span className="badge badge--warn">gated</span>}
          {candidate.llm_ranked && <span className="badge">picked by your local model</span>}
          <span
            className="discover__dot"
            style={{ background: FIT_COLOR[candidate.fit.level] }}
            title={candidate.fit.level === "yellow" || candidate.fit.level === "red" ? candidate.fit.reason : undefined}
          />
        </div>
        <p className="recommend__why">{candidate.why}</p>
        <div className="discover__meta numeric">
          {params(candidate.param_count)} params · ↓ {count(candidate.downloads)} · ♥ {count(candidate.likes)}
          {candidate.last_modified && ` · updated ${candidate.last_modified.slice(0, 7)}`}
        </div>
        {tags.length > 0 && (
          <div className="discover__tags">
            {tags.map((t) => (
              <span key={t} className="chip">
                {t}
              </span>
            ))}
          </div>
        )}

        {open && (
          <div className="discover__files">
            {loading && <p className="muted">Loading files…</p>}
            {err && <p className="import__err">{err}</p>}
            {details?.files
              .filter((f) => f.quant || f.vram_estimate_mb != null)
              .map((f) => (
                <FileRow
                  key={f.path}
                  file={f}
                  gated={candidate.gated}
                  modelType={guessModelType(kind, candidate.format)}
                  roles={rolesFor(kind)}
                />
              ))}
            {details && details.files.every((f) => !f.quant && f.vram_estimate_mb == null) && (
              <p className="muted">No weight files detected in this repo.</p>
            )}
          </div>
        )}
      </div>

      <div className="known__actions">
        <button type="button" onClick={toggle}>
          {open ? "Hide files" : "Files"}
        </button>
      </div>
    </li>
  );
}

/** Hugging Face's own tags for this repo (content descriptors like `nsfw`,
 *  `uncensored`, `roleplay` -- infra noise like `pytorch` or `license:...`
 *  already filtered out), not the local, user-editable tags on the Model
 *  Library table -- those only exist once something is imported. */
function DiscoverTags({ tags }: { tags: string[] }) {
  const shown = filterDisplayTags(tags);
  if (shown.length === 0) return null;
  return (
    <div className="discover__tags">
      {shown.map((t) => (
        <span key={t} className="chip">
          {t}
        </span>
      ))}
    </div>
  );
}

/** Civitai's own malware-scan verdicts for one file, surfaced prominently —
 *  never silently hidden. A non-`"Success"` result (or a scan that hasn't
 *  finished yet) gets a warning badge naming the exact verdict; a clean scan
 *  gets a quiet, low-key note. `null`/`null` (Hugging Face, which runs no
 *  such scan) renders nothing.
 *
 *  This is informational only, deliberately: it does not gate the download
 *  button. AIWM's own import-time Pickle-format guard (`resolve_kind`) is
 *  the real enforcement and runs unconditionally on the actual downloaded
 *  file regardless of what Civitai's self-reported scan claims — trusting
 *  a "Danger" verdict to silently block, same as trusting a "Success"
 *  verdict to silently allow, would both mean trusting Civitai's own
 *  self-report instead of AIWM's own check. Showing the verdict lets the
 *  user make an informed choice; the guard is what actually protects them. */
function ScanBadge({ pickle, virus }: { pickle: string | null; virus: string | null }) {
  if (!pickle && !virus) return null;
  const issues = [
    pickle && pickle !== "Success" ? `pickle: ${pickle}` : null,
    virus && virus !== "Success" ? `virus: ${virus}` : null,
  ].filter((s): s is string => s != null);

  if (issues.length > 0) {
    return (
      <span
        className="badge badge--warn"
        title={`Civitai's own malware scan flagged this file (${issues.join(
          ", ",
        )}). AIWM's own import guard still applies regardless — review before downloading.`}
      >
        ⚠ {issues.join(", ")}
      </span>
    );
  }
  return (
    <span className="discover__scanok" title="Civitai's own malware scan: clean (pickle + virus)">
      scan ok
    </span>
  );
}

/** One resolved file row: fit dot, quant, size, "Download & import", "Copy
 *  link". Shared by Discover's own results and the Models tab's Featured
 *  catalog (`Models.tsx`), which passes `roles` so a coding pick actually
 *  gets the `coding` role on import, and `recommended` to flag the curated
 *  quant among every option Hugging Face offers. */
export function FileRow({
  file,
  gated,
  modelType,
  roles,
  recommended,
}: {
  file: RegistryFile;
  gated: boolean;
  modelType: ModelType;
  /** Roles to stamp on import (e.g. `["chat", "coding"]`); omit for a plain
   *  download. */
  roles?: string[];
  /** Marks this as the curated "pick this one" quant among several shown. */
  recommended?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const [dl, setDl] = useState<"idle" | "queued" | "error">("idle");

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(file.download_url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard blocked — the link is in the title attr */
    }
  };

  const download = async () => {
    setDl("idle");
    try {
      await enqueueDownload({
        url: file.download_url,
        filename: file.path,
        model_type: safeModelType(modelType, file.path),
        sha256: file.sha256 ?? undefined,
        size_bytes: file.size_bytes,
        roles,
      });
      setDl("queued");
    } catch {
      setDl("error");
    }
  };

  const warn = file.fit.level === "yellow" || file.fit.level === "red";
  // Split files need every part — no one-click for those yet (6.4).
  const canDownload = !file.shard && !gated;

  return (
    <div className="discover__file">
      <span
        className="discover__dot"
        style={{ background: FIT_COLOR[file.fit.level] }}
        title={fitTitle(file.fit, file.vram_estimate_mb)}
      />
      <span className="discover__quant">
        {file.quant ?? file.path}
        {file.shard && ` · part ${file.shard[0]}/${file.shard[1]}`}
        {recommended && <span className="badge badge--pick">★</span>}
      </span>
      <span className="numeric muted">{gb(file.size_bytes)}</span>
      {warn && "reason" in file.fit && (
        <span className="discover__fitnote" title={file.fit.reason}>
          {file.fit.level === "red" ? "won’t fit" : "tight"}
        </span>
      )}
      {gated && <span className="badge badge--warn">accept licence on HF</span>}
      <ScanBadge pickle={file.pickle_scan_result} virus={file.virus_scan_result} />
      {canDownload && (
        <button
          type="button"
          className="discover__dlbtn"
          disabled={dl === "queued"}
          onClick={download}
        >
          {dl === "queued" ? "Queued ✓" : dl === "error" ? "Failed — retry" : "Download & import"}
        </button>
      )}
      <button type="button" className="discover__copy" title={file.download_url} onClick={copy}>
        {copied ? "Copied ✓" : "Copy link"}
      </button>
    </div>
  );
}
