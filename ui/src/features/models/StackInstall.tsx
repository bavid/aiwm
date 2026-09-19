import { useEffect, useRef, type FocusEvent, type RefObject } from "react";
import { formatGB } from "../../lib/units";
import type { MemberPhase, StackProgress } from "./stack-install";
import "./stack-install.css";

type Props = {
  /** Short name for the progress bar's accessible label ("WD tagger"). */
  name: string;
  progress: StackProgress | undefined;
  isStarting: boolean;
  /** Why the last click could not start the download. */
  error: string | null;
  /** Why the queue/library could not be read (replaces "Checking…"). */
  loadError: string | null;
  /** The idle button's text, e.g. "Install WD tagger (recommended, 1.3 GB)". */
  installLabel: string;
  /** List every file with its own state while downloading (Models tab). */
  showFiles?: boolean;
  /** The low-emphasis button, for a row inside a list. */
  quiet?: boolean;
  /** Every file is present but the core does not accept the result: say so
   *  and offer this action (via `onInstall`) instead of "Installed". */
  unusable?: Unusable;
  onInstall: () => void;
};

export interface Unusable {
  text: string;
  actionLabel: string;
}

const PHASE_LABEL: Record<MemberPhase, string> = {
  installed: "Installed",
  queued: "Queued",
  downloading: "Downloading",
  paused: "Paused",
  verifying: "Verifying…",
  failed: "Failed",
  missing: "Not downloaded",
};

/** Install button → progress → "Installed" for one catalog stack.
 *
 *  Focus: when the person activates the button, the button is replaced by
 *  the progress bar a moment later. On exactly that render — the button just
 *  unmounted and focus fell back to the page — focus moves to this block
 *  (without scrolling), and the claim is dropped. Polls never move focus.
 *  Screen-reader announcements are the container's job (once, on completion). */
export function StackInstall({
  name,
  progress,
  isStarting,
  error,
  loadError,
  installLabel,
  showFiles = false,
  quiet = false,
  unusable,
  onInstall,
}: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  /** Set by a click; cleared on the render where the button is gone. */
  const restoreFocus = useRef(false);

  useEffect(() => {
    if (!restoreFocus.current || buttonRef.current) return;
    restoreFocus.current = false;
    const active = document.activeElement;
    if (active === null || active === document.body) {
      rootRef.current?.focus({ preventScroll: true });
    }
  });

  // A start that failed puts the button back with an error: the click's
  // claim on focus is over.
  useEffect(() => {
    if (error) restoreFocus.current = false;
  }, [error]);

  const activate = () => {
    restoreFocus.current = true;
    onInstall();
  };
  /** Focus moved on to something else (not the button unmounting, which
   *  blurs with no target): the claim no longer applies. */
  const buttonLeft = (e: FocusEvent<HTMLButtonElement>) => {
    if (e.relatedTarget) restoreFocus.current = false;
  };

  return (
    <div
      className={quiet ? "stackinstall stackinstall--quiet" : "stackinstall"}
      ref={rootRef}
      tabIndex={-1}
    >
      <StackInstallBody
        name={name}
        progress={progress}
        isStarting={isStarting}
        loadError={loadError}
        installLabel={installLabel}
        showFiles={showFiles}
        unusable={unusable}
        buttonRef={buttonRef}
        onActivate={activate}
        onButtonLeft={buttonLeft}
      />
      {error && (
        <p className="stackinstall__err" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

type BodyProps = {
  name: string;
  progress: StackProgress | undefined;
  isStarting: boolean;
  loadError: string | null;
  installLabel: string;
  showFiles: boolean;
  unusable: Unusable | undefined;
  buttonRef: RefObject<HTMLButtonElement | null>;
  onActivate: () => void;
  onButtonLeft: (e: FocusEvent<HTMLButtonElement>) => void;
};

function idleLabel(progress: StackProgress, installLabel: string): string {
  if (progress.phase === "paused") return "Resume download";
  if (progress.phase === "failed") return "Retry download";
  if (progress.installedCount === 0) return installLabel;
  const remaining = progress.members.length - progress.installedCount;
  return `Install the remaining ${remaining} files (${formatGB(progress.bytesTotal - progress.bytesDone)})`;
}

function StackInstallBody({
  name,
  progress,
  isStarting,
  loadError,
  installLabel,
  showFiles,
  unusable,
  buttonRef,
  onActivate,
  onButtonLeft,
}: BodyProps) {
  if (!progress) {
    return loadError ? (
      <p className="stackinstall__err">Could not check what is installed: {loadError}</p>
    ) : (
      <p className="stackinstall__meta">Checking what is installed…</p>
    );
  }

  if (isStarting || progress.phase === "installing") {
    return <StackProgressBar name={name} progress={progress} showFiles={showFiles} />;
  }

  if (progress.phase === "installed") {
    if (!unusable) return <span className="badge badge--installed">Installed ✓</span>;
    return (
      <>
        <p className="stackinstall__err">{unusable.text}</p>
        <button
          type="button"
          className="stackinstall__go"
          ref={buttonRef}
          onClick={onActivate}
          onBlur={onButtonLeft}
        >
          {unusable.actionLabel}
        </button>
      </>
    );
  }

  const failed = progress.members.filter((m) => m.phase === "failed");
  return (
    <>
      <button
        type="button"
        className="stackinstall__go"
        ref={buttonRef}
        onClick={onActivate}
        onBlur={onButtonLeft}
      >
        {idleLabel(progress, installLabel)}
      </button>
      {progress.phase === "paused" && (
        <p className="stackinstall__meta numeric">
          Paused at {Math.round(progress.percent)}% · {formatGB(progress.bytesDone)} of{" "}
          {formatGB(progress.bytesTotal)}
        </p>
      )}
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
  const amount = `${formatGB(progress.bytesDone)} of ${formatGB(progress.bytesTotal)}`;
  const current = progress.current;
  const done = `${progress.installedCount} of ${progress.members.length} files done`;
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
