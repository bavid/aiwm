import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { HelpHint } from "../../components/HelpHint";
import { useAbout, useFeaturedModels, useModelStacks } from "../../lib/hooks";
import {
  enqueueDownload,
  registryModel,
  type FeaturedModel,
  type KnownModel,
  type ModelStack,
  type ModelType,
  type RegistryDetails,
} from "../../lib/ipc";
import { formatGB, formatGiB } from "../../lib/units";
import { FileList } from "./FileList";
import { FitBadge } from "./FitBadge";
import { weightFiles } from "./registry-files";
import { TrainingTools } from "./TrainingTools";

/** Readable names for the kinds whose raw id says little on its own; every
 *  other kind reads fine with its underscores turned into spaces. */
const KIND_LABELS: Partial<Record<ModelType, string>> = {
  wd_tagger: "WD tagger",
  florence2_engine: "Florence-2 file",
  qwen_vl_engine: "Qwen2.5-VL file",
  dia_engine: "Dia engine file",
  dia_codec: "Dia codec file",
};

const kindLabel = (kind: ModelType) => KIND_LABELS[kind] ?? kind.replaceAll("_", " ");

// Values match `KnownModel.media` / `FeaturedModel.role` exactly, so the
// filters below are a plain equality check.
export type CatalogTab = "image" | "video" | "voice" | "training" | "chat" | "coding";
/** These tabs come from the pinned-URL stack catalogue (`GET /models/stacks`);
 *  the rest (`chat`/`coding`) come from the dynamic Featured picks, which
 *  resolve their real file list live against Hugging Face. */
const STACK_TABS: CatalogTab[] = ["image", "video", "voice", "training"];
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
    value: "voice",
    label: "Voice",
    blurb: "The Story Studio narrator — a local text-to-speech model, plus its voice presets.",
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
  {
    value: "training",
    label: "Training & captioning",
    blurb:
      "Captioners for the Dataset tab. They describe every kept frame, so what recurs in your " +
      "frames stays controllable instead of flowing into the trigger word.",
  },
];

type CatalogProps = {
  tab: CatalogTab;
  onTabChange: (tab: CatalogTab) => void;
  /** Bumped by another tab's hand-over: scroll here and focus the active tab. */
  focusRequest: number;
  onUseType: (t: ModelType) => void;
};

/** "What should I install, and for what?" — the curated image/video catalogue
 *  (6.x) plus the chat/coding recommendations, grouped into tabs, each
 *  fit-checked against the current VRAM budget and with one pick per group
 *  flagged "★ recommended for your hardware". */
export function Catalog({ tab, onTabChange, focusRequest, onUseType }: CatalogProps) {
  const stacks = useModelStacks();
  const featured = useFeaturedModels();
  const about = useAbout();
  const sectionRef = useRef<HTMLElement>(null);
  const tabRefs = useRef<Map<CatalogTab, HTMLButtonElement>>(new Map());
  const baseId = useId();
  const tabId = (t: CatalogTab) => `${baseId}-tab-${t}`;
  const panelId = `${baseId}-panel`;

  useEffect(() => {
    if (focusRequest === 0) return;
    sectionRef.current?.scrollIntoView({ block: "start" });
    tabRefs.current.get(tab)?.focus({ preventScroll: true });
    // Only a new request moves focus -- not every tab change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusRequest]);

  const active = CATALOG_TABS.find((t) => t.value === tab)!;
  const isStackTab = STACK_TABS.includes(tab);
  // Memoized: `TrainingTools` watches this array, and a fresh one per render
  // would re-run its completion effect on every poll.
  const stackRows = useMemo(() => stacks?.filter((s) => s.media === tab), [stacks, tab]);
  const featuredRows = featured?.filter((m) => m.role === tab);
  const loading = isStackTab ? !stacks : !featured;

  /** WAI-ARIA tabs: arrows move (and select) with wrap-around, Home/End jump. */
  const onTabKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    const i = CATALOG_TABS.findIndex((t) => t.value === tab);
    const last = CATALOG_TABS.length - 1;
    const moves: Record<string, number> = {
      ArrowRight: i === last ? 0 : i + 1,
      ArrowLeft: i === 0 ? last : i - 1,
      Home: 0,
      End: last,
    };
    const next = moves[e.key] ?? null;
    if (next === null) return;
    e.preventDefault();
    const target = CATALOG_TABS[next].value;
    onTabChange(target);
    tabRefs.current.get(target)?.focus();
  };

  return (
    <section className="card card--wide" ref={sectionRef}>
      <header className="card__head">
        <h2>Recommended models</h2>
        <span className="card__sub">
          {about
            ? `fit-checked against your ~${formatGiB(about.vram_budget_mb, 0)} VRAM budget`
            : "what to install, and for what"}{" "}
          <HelpHint area="models" setting="fit-badge" />
        </span>
      </header>

      <div className="catalog__tabs" role="tablist" aria-label="Model categories">
        {CATALOG_TABS.map((t) => (
          <button
            key={t.value}
            id={tabId(t.value)}
            type="button"
            role="tab"
            aria-selected={tab === t.value}
            aria-controls={panelId}
            tabIndex={tab === t.value ? 0 : -1}
            ref={(el) => {
              if (el) tabRefs.current.set(t.value, el);
              else tabRefs.current.delete(t.value);
            }}
            className={`chip ${tab === t.value ? "chip--on" : ""}`}
            onClick={() => onTabChange(t.value)}
            onKeyDown={onTabKey}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={panelId} aria-labelledby={tabId(tab)}>
        <p className="muted">
          {active.blurb}
          {tab === "training" && (
            <>
              {" "}
              <HelpHint area="models" setting="captioner-stacks" />
            </>
          )}
        </p>

        {loading && <p className="muted">Loading…</p>}
        {!loading && isStackTab && (stackRows?.length ?? 0) === 0 && (
          <p className="muted">Nothing curated here yet.</p>
        )}
        {tab === "training" && stackRows && stackRows.length > 0 && (
          <TrainingTools stacks={stackRows} />
        )}
        {isStackTab && tab !== "training" && stackRows && stackRows.length > 0 && (
          <div className="stacklist">
            {stackRows.map((s) => (
              <StackCard key={s.id} stack={s} onUseType={onUseType} />
            ))}
          </div>
        )}
        {!loading && !isStackTab && (featuredRows?.length ?? 0) === 0 && (
          <p className="muted">Nothing curated here yet.</p>
        )}
        {!isStackTab && featuredRows && featuredRows.length > 0 && (
          <ul className="known">
            {featuredRows.map((m) => (
              <FeaturedRow key={m.id} model={m} onUseType={onUseType} />
            ))}
          </ul>
        )}
      </div>
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
          {model.is_default && <span className="badge badge--pick"><span aria-hidden="true">★</span> recommended</span>}
        </div>
        <span className="known__badges">
          <span className="badge">{kindLabel(model.kind)}</span>
          {model.family && <span className="badge">{model.family}</span>}
          <FitBadge fit={model.fit} subject={model.name} />
        </span>
        {!compact && <span className="known__note">{model.note}</span>}
        <span className="known__file numeric">
          {model.file} · {formatGB(model.size_bytes, 2)} · {model.license}
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
  const downloadAllId = useId();

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
          {stack.is_default && <span className="badge badge--pick"><span aria-hidden="true">★</span> recommended</span>}
        </div>
        <span className="known__badges">
          <span className="badge">
            {stack.members.length} file{stack.members.length > 1 ? "s" : ""}
          </span>
          <span className="badge numeric">{formatGB(totalBytes)} total</span>
          <FitBadge fit={fit} subject={stack.label} />
        </span>
        <span className="known__note">{stack.note}</span>
      </header>

      <ul className="known stackcard__members">
        {stack.members.map((m) => (
          <KnownRow key={m.id} model={m} onUseType={onUseType} compact />
        ))}
      </ul>

      <div className="stackcard__actions">
        <button
          id={downloadAllId}
          type="button"
          onClick={downloadAll}
          disabled={busy || status === "queued"}
        >
          {status === "queued"
            ? `Queued all ${stack.members.length} ✓`
            : status === "error"
              ? "Some failed to queue — check below"
              : busy
                ? "Queuing…"
                : `Download entire stack (${stack.members.length} file${stack.members.length > 1 ? "s" : ""})`}
        </button>
        <HelpHint area="models" setting="stack" describes={downloadAllId} />
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
          {model.is_default && <span className="badge badge--pick"><span aria-hidden="true">★</span> recommended</span>}
        </div>
        <span className="known__badges">
          <span className="badge">{model.role}</span>
          <FitBadge fit={model.fit} subject={model.label} />
        </span>
        <span className="known__note">{model.note}</span>
        <span className="known__file numeric">
          {model.quant_hint} · ~{formatGiB(model.typical_vram_mb)} VRAM (estimate) ·{" "}
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
            {details && (
              <FileList
                files={weightFiles(details)}
                gated={gated}
                modelType="chat"
                roles={model.import_roles}
                isRecommended={(f) =>
                  f.quant?.toUpperCase().includes(hint) ?? f.path.toUpperCase().includes(hint)
                }
                emptyNote="No weight files found right now — open the repo on Hugging Face."
              />
            )}
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
