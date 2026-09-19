import { useId } from "react";
import type { Captioner, ModelStack } from "../../lib/ipc";
import { formatGiB } from "../../lib/units";
import { StackInstall, type Unusable } from "../models/StackInstall";
import { stackSizeLabel } from "../models/stack-install";
import { RECOMMENDED_CAPTIONER_STACK, TRAINING_TOOLS, stackIdForCaptioner } from "../models/training-tools";
import type { StackInstaller } from "../models/useStackInstaller";
import "./captioner-setup.css";

type InstallProps = {
  /** The captioner stacks from the catalog (`null` while loading). */
  stacks: readonly ModelStack[] | null;
  installer: StackInstaller;
  /** Stacks that just finished and await the registry's confirmation. */
  settling: ReadonlySet<string>;
  /** Starts a stack's install — the form records that it was asked here. */
  onInstall: (stack: ModelStack) => void;
  /** Opens the Models tab's Training & captioning section. */
  onMoreCaptioners: () => void;
};

/** Every file of the stack is in the library, yet the core does not call the
 *  captioner usable (e.g. a file failed its load-time integrity check). The
 *  Models card offers the Re-download; this leads there. */
const NOT_USABLE: Unusable = {
  text: "Files present but not usable — reinstall on the Models tab.",
  actionLabel: "Open Models tab",
};

/** One stack's install control, or the "not usable" note when its files are
 *  all there but the registry (the source of truth) says no. */
function CaptionerInstall({
  stack,
  name,
  installLabel,
  quiet,
  installer,
  settling,
  onInstall,
  onMoreCaptioners,
}: {
  stack: ModelStack;
  name: string;
  installLabel: string;
  quiet?: boolean;
  installer: StackInstaller;
  settling: ReadonlySet<string>;
  onInstall: (stack: ModelStack) => void;
  onMoreCaptioners: () => void;
}) {
  const progress = installer.progress.get(stack.id);
  const complete = progress?.phase === "installed";
  if (complete && settling.has(stack.id)) {
    return <p className="stackinstall__meta">Finishing install…</p>;
  }
  return (
    <StackInstall
      name={name}
      progress={progress}
      isStarting={installer.starting.has(stack.id)}
      error={installer.errors[stack.id] ?? null}
      loadError={installer.loadError}
      installLabel={installLabel}
      quiet={quiet}
      unusable={NOT_USABLE}
      onInstall={complete ? onMoreCaptioners : () => onInstall(stack)}
    />
  );
}

/** Shown while no captioner is installed: recommend the WD tagger and
 *  install it right here, through the same download path as the Models tab. */
export function CaptionerHint({
  stacks,
  installer,
  settling,
  onInstall,
  onMoreCaptioners,
}: InstallProps) {
  const wd = stacks?.find((s) => s.id === RECOMMENDED_CAPTIONER_STACK) ?? null;

  return (
    <div className="captioner-hint">
      <p className="datasetform__hint">
        No captioner installed yet. Without one, everything recurring in your frames flows into the
        trigger word.
      </p>
      {wd ? (
        <>
          <p className="captioner-hint__why">
            Recommended: the WD tagger. It writes Danbooru-style tags and runs on the CPU, so it
            never competes with training for VRAM.
          </p>
          <CaptionerInstall
            stack={wd}
            name="WD tagger"
            installLabel={`Install WD tagger (recommended, ${stackSizeLabel(wd)})`}
            installer={installer}
            settling={settling}
            onInstall={onInstall}
            onMoreCaptioners={onMoreCaptioners}
          />
        </>
      ) : (
        stacks !== null && (
          <p className="captioner-hint__why">
            Install one from the Models tab&apos;s Training &amp; captioning section.
          </p>
        )
      )}
      <button type="button" className="captioner-hint__more" onClick={onMoreCaptioners}>
        More captioners…
      </button>
    </div>
  );
}

const runsOn = (c: Captioner) => (c.vram_mb === 0 ? "CPU" : `GPU · ~${formatGiB(c.vram_mb, 0)} VRAM`);

/** "Describe with": every captioner in the registry. `captioner.installed`
 *  (the core's verdict) decides what is selectable; the rest offer their
 *  one-click install instead of sitting there disabled with no explanation. */
export function CaptionerPicker({
  captioners,
  selectedId,
  onSelect,
  stacks,
  installer,
  settling,
  onInstall,
  onMoreCaptioners,
}: InstallProps & {
  captioners: readonly Captioner[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const group = useId();

  return (
    <fieldset className="captioner-picker">
      <legend>Describe with</legend>
      {captioners.map((c) => {
        const meta = `${c.style === "tags" ? "tags" : "prose"} · ${runsOn(c)} · ${c.license}`;
        if (c.installed) {
          return (
            <label key={c.id} className="captioner-picker__row">
              <input
                type="radio"
                name={group}
                value={c.id}
                checked={c.id === selectedId}
                onChange={() => onSelect(c.id)}
              />
              <span className="captioner-picker__text">
                <span className="captioner-picker__name">{c.name}</span>
                <span className="captioner-picker__meta">{meta}</span>
              </span>
            </label>
          );
        }
        const stackId = stackIdForCaptioner(c.id);
        const stack = stacks?.find((s) => s.id === stackId) ?? null;
        const shortName = (stackId && TRAINING_TOOLS[stackId]?.shortName) || c.name;
        return (
          <div key={c.id} className="captioner-picker__row captioner-picker__row--missing">
            <span className="captioner-picker__text">
              <span className="captioner-picker__name">{c.name}</span>
              <span className="captioner-picker__meta">{meta} · not installed</span>
            </span>
            {stack ? (
              <CaptionerInstall
                stack={stack}
                name={shortName}
                installLabel={`Install (${stackSizeLabel(stack)})`}
                quiet
                installer={installer}
                settling={settling}
                onInstall={onInstall}
                onMoreCaptioners={onMoreCaptioners}
              />
            ) : (
              <span className="captioner-picker__meta">Import it on the Models tab.</span>
            )}
          </div>
        );
      })}
    </fieldset>
  );
}
