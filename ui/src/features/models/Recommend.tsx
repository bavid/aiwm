import { useEffect, useState } from "react";
import { useModels } from "../../lib/hooks";
import {
  jobDetail,
  registryModel,
  submitJob,
  type FitVerdict,
  type JobState,
  type ModelType,
  type RecommendCandidate,
  type RecommendKind,
  type RecommendReport,
  type RegistryDetails,
} from "../../lib/ipc";
import { filterDisplayTags } from "../../lib/tags";
import { FileRow } from "./Discover";

const DONE: JobState[] = ["completed", "failed", "cancelled"];

const KINDS: { value: RecommendKind; label: string }[] = [
  { value: "chat", label: "Chat model" },
  { value: "coding", label: "Coding model" },
  { value: "image", label: "Image model" },
  { value: "video", label: "Video model" },
  { value: "lora", label: "LoRA" },
];

const FIT_COLOR: Record<FitVerdict["level"], string> = {
  green: "var(--load-ok)",
  yellow: "var(--load-warn)",
  red: "var(--load-crit)",
  unknown: "var(--border)",
};

const count = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : `${n}`;
const params = (n: number | null) =>
  n == null ? "—" : n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : `${(n / 1e6).toFixed(0)}M`;

/** The user's chosen kind is a far better signal than the file format alone
 *  (`Discover.tsx`'s generic guess only has format to go on) -- the user can
 *  still correct it via "Set import type" on the expanded file list. */
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

/** "Find a model" — describe what you want in plain language, get a
 *  shortlist from a live Hugging Face search that your own local chat model
 *  has picked through and explained. Sibling to `UpgradeChecks` ("is there
 *  something better than what I have"), but framed as a fresh suggestion
 *  from a description instead of a comparison to an installed model. */
export function Recommend() {
  const { data: models } = useModels();
  const reasonerModels = (models ?? []).filter(
    (m) => m.roles.includes("chat") || m.roles.includes("coding"),
  );

  const [query, setQuery] = useState("");
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
      if (!DONE.includes(job.state)) return;
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
    <section className="card card--wide recommend">
      <header className="card__head">
        <h2>Find a model</h2>
        <span className="card__sub">describe what you want, get a shortlist</span>
      </header>

      <form
        className="recommend__form"
        onSubmit={(e) => {
          e.preventDefault();
          search();
        }}
      >
        <input
          className="recommend__query"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="e.g. realistic uncensored nsfw, or best local coding model"
          spellCheck
        />
        <select value={kind} onChange={(e) => setKind(e.target.value as RecommendKind)}>
          {KINDS.map((k) => (
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
      </form>

      {error && <p className="import__err">{error}</p>}
      {report && (
        <>
          <p className="muted">{report.note}</p>
          {report.freshness.kind !== "live" && (
            <p className="muted">
              {report.freshness.kind === "offline" ? "Offline — cached results." : "Hugging Face was unreachable — showing the last cached results."}
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
    </section>
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
