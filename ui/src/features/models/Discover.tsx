import { useEffect, useMemo, useState } from "react";
import {
  enqueueDownload,
  registryModel,
  type FitVerdict,
  type Freshness,
  type ModelType,
  type RegistryDetails,
  type RegistryFile,
  type RegistrySearchParams,
  type RemoteModel,
} from "../../lib/ipc";
import { useRegistrySearch } from "../../lib/hooks";

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

function fitTitle(fit: FitVerdict, vramMb: number | null): string {
  const est = vramMb ? ` · ~${(vramMb / 1024).toFixed(1)} GB VRAM` : "";
  if (fit.level === "green") return `Fits comfortably${est}`;
  if (fit.level === "unknown") return "Fit unknown";
  return `${fit.reason}${est}`;
}

function freshnessNote(f: Freshness): string | null {
  if (f.kind === "live") return null;
  const mins = Math.max(1, Math.round(f.age_secs / 60));
  return f.kind === "offline"
    ? `Offline — showing a cached result from ~${mins} min ago.`
    : `Hugging Face was unreachable — showing a cached result from ~${mins} min ago.`;
}

/** The "Discover" panel: search the Hugging Face Hub. Per file row: "Download &
 *  import" (the 6.4 download manager — disabled for split GGUFs and gated repos),
 *  "Copy link", and "Set import type". */
export function Discover({ onUseType }: { onUseType: (t: ModelType) => void }) {
  const [text, setText] = useState("");
  const [gguf, setGguf] = useState(true);
  const [sort, setSort] = useState<NonNullable<RegistrySearchParams["sort"]>>("downloads");

  const trimmed = text.trim();
  const enabled = trimmed.length >= 2;
  const query = useMemo<RegistrySearchParams>(
    () => ({ q: trimmed || undefined, gguf, sort, limit: 20 }),
    [trimmed, gguf, sort],
  );
  const { result, error, loading } = useRegistrySearch(query, enabled);
  const note = result ? freshnessNote(result.freshness) : null;

  const [recent, setRecent] = useState<string[]>(loadRecent);
  useEffect(() => {
    if (enabled && result && result.data.length > 0) {
      setRecent(pushRecent(trimmed));
    }
  }, [enabled, result, trimmed]);

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Discover models</h2>
        <span className="card__sub">search Hugging Face · download in your browser, then import above</span>
      </header>

      <div className="discover__controls">
        <input
          type="text"
          className="discover__search"
          value={text}
          placeholder="qwen2.5 coder, flux, whisper…"
          spellCheck={false}
          onChange={(e) => setText(e.target.value)}
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
            <button key={t} type="button" className="chip" onClick={() => setText(t)}>
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
    </section>
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
        model_type: modelType,
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
