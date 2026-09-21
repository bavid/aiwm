import { useEffect, useId, useRef, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import {
  enqueueDownload,
  type DownloadOrigin,
  type ModelType,
  type RegistryFile,
} from "../../lib/ipc";
import { formatGB } from "../../lib/units";
import { FitBadge } from "./FitBadge";
import { countFitTiers, fitTierSummary, sortByFitTier } from "./fit-utils";

/** How long "Copied ✓" / "Queued ✓" style feedback stays up. */
const COPIED_MS = 1500;

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

/** The resolved file list of one model: every weight file the source offers,
 *  sorted into hardware-fit tiers, with a one-line tier summary and an opt-in
 *  filter for the ones that won't fit.
 *
 *  The filter is off by default and, when on, always says how many rows it is
 *  holding back — a file list that quietly drops options is worse than one
 *  that shows a red badge. Nothing here blocks a download: "Too big" is an
 *  estimate about the current VRAM budget, and downloading anyway is a
 *  legitimate choice (a bigger GPU next month, CPU offloading, plain
 *  curiosity). */
export function FileList({
  files,
  gated,
  modelType,
  roles,
  origin,
  isRecommended,
  emptyNote,
}: {
  files: RegistryFile[];
  gated: boolean;
  modelType: ModelType;
  /** Roles to stamp on import (e.g. `["chat", "coding"]`); omit for a plain
   *  download. */
  roles?: string[];
  /** Where the files come from — recorded on the import with the base
   *  family its label maps to. */
  origin?: DownloadOrigin;
  /** Flags the curated "pick this one" quant among several shown. */
  isRecommended?: (file: RegistryFile) => boolean;
  /** Shown instead of the list when the source returned no usable file. */
  emptyNote: string;
}) {
  const [hideTooBig, setHideTooBig] = useState(false);
  const hideId = useId();

  if (files.length === 0) return <p className="muted">{emptyNote}</p>;

  const counts = countFitTiers(files);
  const sorted = sortByFitTier(files);
  const shown = hideTooBig ? sorted.filter((f) => f.fit.level !== "red") : sorted;
  const hidden = sorted.length - shown.length;
  const summary = fitTierSummary(counts);

  return (
    <>
      <div className="filelist__head">
        {summary && <span className="filelist__summary">{summary}</span>}
        {/* Only offered when there is actually something to hide — an inert
            checkbox on a list where every file fits is just noise. */}
        {counts.red > 0 && (
          <span className="filelist__filter">
            <span className="chip">
              <input
                id={hideId}
                type="checkbox"
                checked={hideTooBig}
                onChange={(e) => setHideTooBig(e.target.checked)}
              />
              <label htmlFor={hideId}>Hide files that won’t fit</label>
            </span>
            <HelpHint area="models" setting="hide-too-big" describes={hideId} />
          </span>
        )}
      </div>

      {shown.map((f) => (
        <FileRow
          key={f.path}
          file={f}
          gated={gated}
          modelType={modelType}
          roles={roles}
          origin={origin}
          recommended={isRecommended?.(f)}
        />
      ))}

      {/* The live region exists from the first render — a `role="status"`
          element that only appears together with its text is often created
          too late for the announcement to be made at all. */}
      <p className="muted filelist__hidden" role="status">
        {hidden === 0
          ? ""
          : hidden === 1
            ? "1 file hidden because it won’t fit on this GPU — untick the filter to show it."
            : `${hidden} files hidden because they won’t fit on this GPU — untick the filter to show them.`}
      </p>
    </>
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
 *  user make an informed choice; the guard is what actually protects them.
 *
 *  What the badge means is real (visually-hidden) text, not a `title=`
 *  tooltip — a tooltip never reaches keyboard or touch users. The full
 *  explanation is the `scan` entry of the Models help. */
function ScanBadge({ pickle, virus }: { pickle: string | null; virus: string | null }) {
  if (!pickle && !virus) return null;
  const issues = [
    pickle && pickle !== "Success" ? `pickle: ${pickle}` : null,
    virus && virus !== "Success" ? `virus: ${virus}` : null,
  ].filter((s): s is string => s != null);

  if (issues.length > 0) {
    return (
      <span className="badge badge--warn">
        <span aria-hidden="true">⚠ </span>
        {issues.join(", ")}
        <span className="visually-hidden">
          {" "}
          — Civitai's own malware scan flagged this file; AIWM's import guard still applies.
          Review before downloading.
        </span>
      </span>
    );
  }
  return (
    <span className="discover__scanok">
      scan ok
      <span className="visually-hidden"> — Civitai's own malware scan: clean (pickle and virus)</span>
    </span>
  );
}

/** One resolved file row: fit badge, quant, size, "Download & import", "Copy
 *  link". `roles` are stamped on import (e.g. `["chat", "coding"]`, so a
 *  coding pick actually gets the `coding` role); `recommended` flags the
 *  curated quant among every option the source offers. */
function FileRow({
  file,
  gated,
  modelType,
  roles,
  origin,
  recommended,
}: {
  file: RegistryFile;
  gated: boolean;
  modelType: ModelType;
  roles?: string[];
  origin?: DownloadOrigin;
  recommended?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const [dl, setDl] = useState<"idle" | "queued" | "error">("idle");
  const copiedTimer = useRef<number | null>(null);

  // The row can disappear while the "Copied ✓" timer is still pending (the
  // fit filter, a collapsed card, a new search) — drop the timer with it.
  useEffect(
    () => () => {
      if (copiedTimer.current != null) window.clearTimeout(copiedTimer.current);
    },
    [],
  );

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(file.download_url);
      setCopied(true);
      if (copiedTimer.current != null) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
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
        origin,
      });
      setDl("queued");
    } catch {
      setDl("error");
    }
  };

  const name = file.quant ?? file.path;
  // Split files need every part — no one-click for those yet (6.4).
  const canDownload = !file.shard && !gated;

  return (
    <div className="discover__file">
      <FitBadge fit={file.fit} vramMb={file.vram_estimate_mb} subject={name} />
      <span className="discover__quant">
        {name}
        {file.shard && ` · part ${file.shard[0]}/${file.shard[1]}`}
        {recommended && <span className="badge badge--pick">★</span>}
      </span>
      <span className="numeric muted">{formatGB(file.size_bytes, 2)}</span>
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
