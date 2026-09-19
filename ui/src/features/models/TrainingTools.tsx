import { useCallback, useEffect, useId, useState } from "react";
import { useCaptioners } from "../../lib/hooks";
import type { ModelStack } from "../../lib/ipc";
import { FitBadge } from "./FitBadge";
import { StackInstall, type Unusable } from "./StackInstall";
import { stackSizeLabel, type StackProgress } from "./stack-install";
import { TRAINING_TOOLS, stackIdForCaptioner } from "./training-tools";
import { useSettling } from "./useSettling";
import { useStackInstaller } from "./useStackInstaller";

/** The catalog's "Training & captioning" tab: the dataset captioners as
 *  one-click stacks, each with what it is for, where it runs, its size and
 *  licence, its fit, and whether it is installed. The fit comes from the
 *  core's `stack_fit`, which judges a captioner stack by the VRAM it actually
 *  reserves at run time (WD tagger: none, CPU; Florence-2: ~2 GiB; Qwen2.5-VL
 *  loaded 4-bit: ~6 GiB) — not by summing its file sizes. */
export function TrainingTools({ stacks }: { stacks: readonly ModelStack[] }) {
  const [announcement, setAnnouncement] = useState("");
  const { data: captioners, refetch: refetchCaptioners } = useCaptioners();
  const { ids: settling, settle, clear } = useSettling();
  const onInstalled = useCallback(
    (s: ModelStack) => {
      const name = TRAINING_TOOLS[s.id]?.shortName ?? s.label;
      setAnnouncement(`${name} installed.`);
      settle(s.id);
      refetchCaptioners();
    },
    [settle, refetchCaptioners],
  );
  const installer = useStackInstaller(stacks, onInstalled);

  // The registry confirmed a just-finished captioner: its settle window ends.
  useEffect(() => {
    for (const c of captioners ?? []) {
      const stackId = stackIdForCaptioner(c.id);
      if (c.installed && stackId) clear(stackId);
    }
  }, [captioners, clear]);

  /** Complete files the core still does not accept as a usable captioner
   *  (after the settle window) — offer to fetch them again. */
  const unusableFor = (s: ModelStack): Unusable | undefined => {
    const captionerId = TRAINING_TOOLS[s.id]?.captionerId;
    const captioner = captioners?.find((c) => c.id === captionerId);
    if (!captioner || captioner.installed || settling.has(s.id)) return undefined;
    return {
      text: "Files present but not usable — the core rejected them. Fetch them again to restore the pinned files.",
      actionLabel: `Re-download (${stackSizeLabel(s)})`,
    };
  };

  return (
    <>
      <div className="stacklist">
        {stacks.map((s) => (
          <TrainingToolCard
            key={s.id}
            stack={s}
            progress={installer.progress.get(s.id)}
            isStarting={installer.starting.has(s.id)}
            error={installer.errors[s.id] ?? null}
            loadError={installer.loadError}
            unusable={unusableFor(s)}
            onInstall={() => void installer.install(s)}
            onRedownload={() => void installer.redownload(s)}
          />
        ))}
      </div>
      <p className="visually-hidden" role="status">
        {announcement}
      </p>
    </>
  );
}

function TrainingToolCard({
  stack,
  progress,
  isStarting,
  error,
  loadError,
  unusable,
  onInstall,
  onRedownload,
}: {
  stack: ModelStack;
  progress: StackProgress | undefined;
  isStarting: boolean;
  error: string | null;
  loadError: string | null;
  unusable: Unusable | undefined;
  onInstall: () => void;
  onRedownload: () => void;
}) {
  const headingId = useId();
  const info = TRAINING_TOOLS[stack.id];
  const size = stackSizeLabel(stack);
  const licenses = [...new Set(stack.members.map((m) => m.license))].join(", ");
  const fileCount = stack.members.length;
  const shortName = info?.shortName ?? stack.label;

  return (
    <section className="stackcard toolcard" aria-labelledby={headingId}>
      <header className="stackcard__head">
        <div className="known__name">
          <h3 id={headingId} className="toolcard__title">
            {stack.label}
          </h3>
          {stack.is_default && (
            <span className="badge badge--pick">
              <span aria-hidden="true">★</span> Recommended
            </span>
          )}
        </div>
        {info && <p className="toolcard__purpose">{info.purpose}</p>}
        <span className="known__badges">
          {info && <span className="badge">Runs on {info.runsOn}</span>}
          <span className="badge numeric">{size}</span>
          <span className="badge">{licenses}</span>
          <span className="badge numeric">
            {fileCount} file{fileCount > 1 ? "s" : ""}
          </span>
          <FitBadge fit={stack.fit} subject={stack.label} />
        </span>
        <span className="known__note">{stack.note}</span>
      </header>
      <StackInstall
        name={shortName}
        progress={progress}
        isStarting={isStarting}
        error={error}
        loadError={loadError}
        unusable={unusable}
        installLabel={`Install ${shortName} (${size})`}
        showFiles
        onInstall={unusable && progress?.phase === "installed" ? onRedownload : onInstall}
      />
    </section>
  );
}
