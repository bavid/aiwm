import { useEffect, useId, useMemo, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { Lightbox } from "../../components/Lightbox";
import { useCivitaiSearch, useCivitaiStatus, useModels, useRegistrySearch } from "../../lib/hooks";
import {
  cancelJob,
  civitaiModel,
  jobDetail,
  registryModel,
  submitJob,
  type CivitaiSearchParams,
  type Freshness,
  type JobState,
  type ModelType,
  type RecommendCandidate,
  type RecommendKind,
  type RecommendReport,
  type RegistryDetails,
  type RegistrySearchParams,
  type RemoteModel,
  type RemotePreview,
} from "../../lib/ipc";
import { filterDisplayTags } from "../../lib/tags";
import { FileList } from "./FileList";
import { FitBadge } from "./FitBadge";
import { weightFiles } from "./registry-files";

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

const count = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : `${n}`;
const params = (n: number | null) => (n == null ? "—" : n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : `${(n / 1e6).toFixed(0)}M`);

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
  const ids = useId();
  const sourceGroupId = `${ids}-source`;
  const aiModeId = `${ids}-ai`;

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

      <div className="discover__sources">
        <div
          id={sourceGroupId}
          className="catalog__tabs"
          role="tablist"
          aria-label="Discover source"
        >
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
        <HelpHint area="models" setting="source" describes={sourceGroupId} />
      </div>

      {source === "huggingface" && (
        <>
          <span className="discover__aitoggle">
            <span className="chip">
              <input
                id={aiModeId}
                type="checkbox"
                checked={aiMode}
                onChange={(e) => setAiMode(e.target.checked)}
              />
              <label htmlFor={aiModeId}>Ask my local model to rank &amp; explain</label>
            </span>
            <HelpHint area="models" setting="ai-rank" describes={aiModeId} />
          </span>

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
  const ids = useId();
  const ggufId = `${ids}-gguf`;
  const sortId = `${ids}-sort`;

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
          aria-label="Search Hugging Face"
          value={query}
          placeholder="qwen2.5 coder, flux, whisper…"
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="chip">
          <input
            id={ggufId}
            type="checkbox"
            checked={gguf}
            onChange={(e) => setGguf(e.target.checked)}
          />
          <label htmlFor={ggufId}>GGUF only</label>
        </span>
        <HelpHint area="models" setting="gguf-only" describes={ggufId} />
        <select
          id={sortId}
          aria-label="Sort"
          value={sort}
          onChange={(e) => setSort(e.target.value as typeof sort)}
        >
          {SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        <HelpHint area="models" setting="sort" describes={sortId} />
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
  const ids = useId();
  const kindId = `${ids}-kind`;
  const reasonerFieldId = `${ids}-reasoner`;
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
          aria-label="What are you looking for"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="e.g. realistic uncensored nsfw, or best local coding model"
          spellCheck
        />
        <select
          id={kindId}
          aria-label="Kind of model"
          value={kind}
          onChange={(e) => setKind(e.target.value as RecommendKind)}
        >
          {RECOMMEND_KINDS.map((k) => (
            <option key={k.value} value={k.value}>
              {k.label}
            </option>
          ))}
        </select>
        <HelpHint area="models" setting="kind" describes={kindId} />
        <select
          id={reasonerFieldId}
          value={reasonerId}
          onChange={(e) => setReasonerId(e.target.value)}
          aria-label="Which local model reasons about the results"
        >
          <option value="auto">Auto (most-recently-used)</option>
          {reasonerModels.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
        <HelpHint area="models" setting="reasoner" describes={reasonerFieldId} />
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

/** A pasted model link — or a bare model id — goes straight to that model
 *  instead of through the search.
 *
 *  Civitai's `query` is name-matching, not full-text: measured 2026-09-20,
 *  "Realism by Stable Yogi Krea2" (id 2786499) is the 4th hit for `realism`
 *  but is nowhere in the first 20 for `krea`, and the API answers one page at
 *  a time. Its own id answers instantly, and a link is what you have in hand
 *  when you found the model in a browser. Both hosts are accepted, with or
 *  without the `?modelVersionId=` the site appends. */
const CIVITAI_LINK = /civitai\.(?:com|red)\/models\/(\d+)/i;
const directModelId = (query: string): string | null => {
  const q = query.trim();
  const fromLink = CIVITAI_LINK.exec(q);
  if (fromLink) return fromLink[1];
  return /^\d{3,}$/.test(q) ? q : null;
};

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
  const nsfwId = useId();

  // Which front door this app is on (Settings → Network & API). The model
  // page lives on the same host, and an adult model is not on the
  // safe-for-work one, so linking there would 404 for the user.
  const civitaiStatus = useCivitaiStatus();
  const host = civitaiStatus.data?.base_url || "https://civitai.com";

  const trimmed = query.trim();
  const linkedId = directModelId(trimmed);
  const [linked, setLinked] = useState<RegistryDetails | null>(null);
  const [linkedError, setLinkedError] = useState<string | null>(null);

  useEffect(() => {
    if (!linkedId) {
      setLinked(null);
      setLinkedError(null);
      return;
    }
    let alive = true;
    setLinked(null);
    setLinkedError(null);
    civitaiModel(linkedId)
      .then((d) => alive && setLinked(d))
      .catch((e) => alive && setLinkedError(e instanceof Error ? e.message : String(e)));
    return () => {
      alive = false;
    };
  }, [linkedId]);

  const searchParams = useMemo<CivitaiSearchParams>(
    () => ({ q: trimmed || undefined, types, sort, nsfw, limit: 20 }),
    [trimmed, types, sort, nsfw],
  );
  const { result, error, loading } = useCivitaiSearch(searchParams, !linkedId);
  const note = result ? freshnessNote(result.freshness, "Civitai") : null;

  const toggleType = (t: string) =>
    setTypes((prev) => (prev.includes(t) ? prev.filter((x) => x !== t) : [...prev, t]));

  return (
    <>
      <div className="discover__controls">
        <input
          type="text"
          className="discover__search"
          aria-label="Search Civitai"
          value={query}
          placeholder="pony, realistic, anime style… or paste a model link"
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
        <select
          aria-label="Sort"
          value={sort}
          onChange={(e) => setSort(e.target.value as typeof sort)}
        >
          {CIVITAI_SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        <span className="chip">
          <input
            id={nsfwId}
            type="checkbox"
            checked={nsfw}
            onChange={(e) => setNsfw(e.target.checked)}
          />
          <label htmlFor={nsfwId}>Show NSFW</label>
        </span>
        <HelpHint area="models" setting="nsfw" describes={nsfwId} />
      </div>

      {linkedId ? (
        <>
          <p className="muted">
            Showing model {linkedId} from the link you pasted — the type and sort above do not
            apply to it.
          </p>
          {linkedError && <p className="import__err">{linkedError}</p>}
          {!linked && !linkedError && <p className="muted">Loading that model…</p>}
        </>
      ) : (
        <>
          {loading && !result && <p className="muted">Searching…</p>}
          {error && <p className="import__err">{error}</p>}
          {note && <p className="discover__stale">{note}</p>}
          {types.length === 0 && (
            <p className="muted">Pick at least one type (Checkpoints / LoRAs) to search.</p>
          )}
          {result && result.data.length === 0 && !loading && types.length > 0 && (
            <p className="muted">
              No models match — try a broader term, turn on “Show NSFW”, or paste the model’s
              link from Civitai. Search matches names and shows the first 20 hits, so a model
              can sit just outside them.
            </p>
          )}
        </>
      )}

      <ul className="discover__results">
        {linkedId
          ? linked && (
              <CivitaiResultCard
                key={linked.id}
                model={linked}
                onUseType={onUseType}
                showNsfw={nsfw}
                host={host}
              />
            )
          : result?.data.map((m) => (
              <CivitaiResultCard
                key={m.id}
                model={m}
                onUseType={onUseType}
                showNsfw={nsfw}
                host={host}
              />
            ))}
      </ul>
    </>
  );
}

/** A result row's sample gallery: the strip stays closed until asked for, so
 *  opening the Discover tab still loads one thumbnail per row and not a dozen.
 *  Samples rated above `1` are Civitai's own "spicier than the model's own
 *  flag" marker and stay hidden unless the search is already showing adult
 *  content. Clicking one opens the shared Lightbox, arrows included. */
function PreviewStrip({ previews, showNsfw }: { previews: RemotePreview[]; showNsfw: boolean }) {
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState<number | null>(null);

  const shown = showNsfw ? previews : previews.filter((p) => p.nsfw_level <= 1);
  const hidden = previews.length - shown.length;
  if (previews.length <= 1) return null;

  const current = at !== null ? shown[at] : null;

  return (
    <div className="discover__gallery">
      <button type="button" className="chip" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? "Hide previews" : `Previews (${shown.length})`}
      </button>
      {hidden > 0 && (
        <span className="muted">
          {hidden} hidden — tick “Show NSFW” to include {hidden === 1 ? "it" : "them"}
        </span>
      )}
      {open && (
        <ul className="discover__strip">
          {shown.map((p, i) => (
            <li key={p.url}>
              <button
                type="button"
                className="discover__thumb"
                aria-label={`Open preview ${i + 1} of ${shown.length}`}
                onClick={() => setAt(i)}
              >
                {p.is_video ? (
                  <video src={p.url} muted playsInline preload="metadata" width={72} height={72} />
                ) : (
                  <img src={p.url} alt="" width={72} height={72} loading="lazy" />
                )}
                {p.is_video && <span className="discover__play">▶</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
      {current && (
        <Lightbox
          kind={current.is_video ? "video" : "image"}
          src={current.url}
          caption={`Preview ${(at ?? 0) + 1} of ${shown.length}`}
          onClose={() => setAt(null)}
          onPrev={at !== null && at > 0 ? () => setAt(at - 1) : undefined}
          onNext={at !== null && at < shown.length - 1 ? () => setAt(at + 1) : undefined}
        />
      )}
    </div>
  );
}

function CivitaiResultCard({
  model,
  onUseType,
  showNsfw,
  host,
}: {
  model: RemoteModel;
  onUseType: (t: ModelType) => void;
  /** Whether the search that produced this row is showing adult content. */
  showNsfw: boolean;
  /** The front door the app is configured for — the model page lives on the
   *  same host, and an adult model is not on the safe-for-work one. */
  host: string;
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
            href={`${host}/models/${model.id}`}
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

        <PreviewStrip previews={model.previews} showNsfw={showNsfw} />

        {open && (
          <div className="discover__files">
            {loading && <p className="muted">Loading files…</p>}
            {err && <p className="import__err">{err}</p>}
            {details && (
              <FileList
                files={details.files}
                gated={false}
                modelType={modelType}
                emptyNote="No files listed for this model."
              />
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
            {details && (
              <FileList
                files={weightFiles(details)}
                gated={model.gated !== "no"}
                modelType={importTypeFor(model.format)}
                emptyNote="No weight files detected in this repo."
              />
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
          <FitBadge fit={candidate.fit} subject={candidate.id} />
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
            {details && (
              <FileList
                files={weightFiles(details)}
                gated={candidate.gated}
                modelType={guessModelType(kind, candidate.format)}
                roles={rolesFor(kind)}
                emptyNote="No weight files detected in this repo."
              />
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
