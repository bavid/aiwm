import { useEffect, useRef, type FocusEvent } from "react";
import { decimalGb, type MemberPhase, type StackProgress } from "./stack-install";
import "./stack-install.css";

type Props = {
  /** Short name for the progress bar's accessible label ("WD tagger"). */
  name: string;
  progress: StackProgress | undefined;
  isStarting: boolean;
  /** Why the last click could not start the download, shown verbatim. */
  error: string | null;
  /** The idle button's text, e.g. "Install WD tagger (recommended, 1.3 GB)". */
  installLabel: string;
  /** List every file with its own state while downloading (Models tab). */
  showFiles?: boolean;
  /** The low-emphasis button, for a row inside a list. */
  quiet?: boolean;
  onInstall: () => void;
};

const PHASE_LABEL: Record<MemberPhase, string> = {
  installed: "Installed",
  queued: "Queued",
  downloading: "Downloading",
  paused: "Paused",
  verifying: "Verifying…",
  failed: "Failed",
  missing: "Not downloaded",
};

/** Install button → progress → "Installed" for one catalog stack. Owns only
 *  focus: when the button it replaced had focus, focus moves to this block
 *  instead of falling back to the page, so a keyboard user keeps their place
 *  through every state change. Screen-reader announcements are the
 *  container's job (once, on completion). */
export function StackInstall({
  name,
  progress,
  isStarting,
  error,
  installLabel,
  showFiles = false,
  quiet = false,
  onInstall,
}: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const ownsFocus = useRef(false);

  useEffect(() => {
    if (ownsFocus.current && document.activeElement === document.body) {
      rootRef.current?.focus();
    }
  });

  const onFocus = () => {
    ownsFocus.current = true;
  };
  const onBlur = (e: FocusEvent<HTMLDivElement>) => {
    // A null `relatedTarget` is the focused button unmounting — keep the
    // claim so the effect can pull focus back here.
    const to = e.relatedTarget;
    if (to && !e.currentTarget.contains(to)) ownsFocus.current = false;
  };

  return (
    <div
      className={quiet ? "stackinstall stackinstall--quiet" : "stackinstall"}
      ref={rootRef}
      tabIndex={-1}
      onFocus={onFocus}
      onBlur={onBlur}
    >
      <StackInstallBody
        name={name}
        progress={progress}
        isStarting={isStarting}
        installLabel={installLabel}
        showFiles={showFiles}
        onInstall={onInstall}
      />
      {error && (
        <p className="stackinstall__err" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function StackInstallBody({
  name,
  progress,
  isStarting,
  installLabel,
  showFiles,
  onInstall,
}: Omit<Props, "error" | "quiet" | "showFiles"> & { showFiles: boolean }) {
  if (!progress) return <p className="stackinstall__meta">Checking what is installed…</p>;

  if (progress.phase === "installed") {
    return <span className="badge badge--installed">Installed ✓</span>;
  }

  if (isStarting || progress.phase === "installing") {
    return <StackProgressBar name={name} progress={progress} showFiles={showFiles} />;
  }

  const failed = progress.members.filter((m) => m.phase === "failed");
  const remaining = progress.bytesTotal - progress.bytesDone;
  const label =
    failed.length > 0
      ? "Retry download"
      : progress.installedCount > 0
        ? `Install the remaining ${progress.members.length - progress.installedCount} files (${decimalGb(remaining)})`
        : installLabel;

  return (
    <>
      <button type="button" className="stackinstall__go" onClick={onInstall}>
        {label}
      </button>
      {failed.map((m) => (
        <p key={m.member.id} className="stackinstall__err">
          {m.member.file}: {m.error ?? "download failed"}
        </p>
      ))}
    </>
  );
}

function StackProgressBar({
  name,
  progress,
  showFiles,
}: {
  name: string;
  progress: StackProgress;
  showFiles: boolean;
}) {
  const pct = Math.round(progress.percent);
  const amount = `${decimalGb(progress.bytesDone)} of ${decimalGb(progress.bytesTotal)}`;
  const current = progress.current;
  const fileCount = progress.members.length;
  const done = `${progress.installedCount} of ${fileCount} files done`;
  const summary = current ? `${PHASE_LABEL[current.phase]} ${current.member.file} · ${done}` : "Starting…";

  return (
    <div className="stackinstall__progress">
      <div
        className="stackinstall__bar"
        role="progressbar"
        aria-label={`Downloading ${name}`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        aria-valuetext={`${pct}% — ${amount}`}
      >
        <div className="stackinstall__fill" style={{ width: `${pct}%` }} />
      </div>
      <p className="stackinstall__meta numeric">
        {pct}% · {amount} · {summary}
      </p>
      {showFiles && (
        <ul className="stackinstall__files">
          {progress.members.map((m) => (
            <li key={m.member.id} className={`stackinstall__file stackinstall__file--${m.phase}`}>
              <span className="stackinstall__filename">{m.member.file}</span>
              <span className="numeric">
                {PHASE_LABEL[m.phase]}
                {m.phase === "downloading" &&
                  ` · ${Math.round((m.bytesDone / Math.max(1, m.member.size_bytes)) * 100)}%`}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
