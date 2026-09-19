import { useCallback, useId, useState } from "react";
import type { ModelStack } from "../../lib/ipc";
import { FitBadge } from "./FitBadge";
import { StackInstall } from "./StackInstall";
import { stackSizeLabel, type StackProgress } from "./stack-install";
import { TRAINING_TOOLS } from "./training-tools";
import { useStackInstaller } from "./useStackInstaller";

/** The catalog's "Training & captioning" tab: the dataset captioners as
 *  one-click stacks, each with what it is for, where it runs, its size and
 *  licence, its fit, and whether it is installed. The fit comes from the
 *  core's `stack_fit`, which judges a captioner stack by the VRAM it actually
 *  reserves at run time (WD tagger: none, CPU; Florence-2: ~2 GiB; Qwen2.5-VL
 *  loaded 4-bit: ~6 GiB) — not by summing its file sizes. */
export function TrainingTools({ stacks }: { stacks: readonly ModelStack[] }) {
  const [announcement, setAnnouncement] = useState("");
  const onInstalled = useCallback((s: ModelStack) => {
    const name = TRAINING_TOOLS[s.id]?.shortName ?? s.label;
    setAnnouncement(`${name} installed.`);
  }, []);
  const installer = useStackInstaller(stacks, onInstalled);

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
            onInstall={() => void installer.install(s)}
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
  onInstall,
}: {
  stack: ModelStack;
  progress: StackProgress | undefined;
  isStarting: boolean;
  error: string | null;
  loadError: string | null;
  onInstall: () => void;
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
        installLabel={`Install ${shortName} (${size})`}
        showFiles
        onInstall={onInstall}
      />
    </section>
  );
}
