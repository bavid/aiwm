import { useId } from "react";
import type { Captioner, ModelStack } from "../../lib/ipc";
import { StackInstall } from "../models/StackInstall";
import { decimalGb, stackBytes } from "../models/stack-install";
import { RECOMMENDED_CAPTIONER_STACK, TRAINING_TOOLS, stackIdForCaptioner } from "../models/training-tools";
import type { StackInstaller } from "../models/useStackInstaller";
import "./captioner-setup.css";

type InstallProps = {
  /** The captioner stacks from the catalog (`null` while loading). */
  stacks: readonly ModelStack[] | null;
  installer: StackInstaller;
};

/** Shown while no captioner is installed: recommend the WD tagger and
 *  install it right here, through the same download path as the Models tab. */
export function CaptionerHint({
  stacks,
  installer,
  onMoreCaptioners,
}: InstallProps & { onMoreCaptioners: () => void }) {
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
          <StackInstall
            name="WD tagger"
            progress={installer.progress.get(wd.id)}
            isStarting={installer.starting.has(wd.id)}
            error={installer.errors[wd.id] ?? null}
            installLabel={`Install WD tagger (recommended, ${decimalGb(stackBytes(wd))})`}
            onInstall={() => void installer.install(wd)}
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

const runsOn = (c: Captioner) =>
  c.vram_mb === 0 ? "CPU" : `GPU · ~${(c.vram_mb / 1024).toFixed(0)} GB VRAM`;

/** "Describe with": every captioner in the registry. Installed ones are
 *  selectable; the rest offer their one-click install instead of sitting
 *  there disabled with no explanation. */
export function CaptionerPicker({
  captioners,
  selectedId,
  onSelect,
  stacks,
  installer,
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
              <StackInstall
                name={shortName}
                progress={installer.progress.get(stack.id)}
                isStarting={installer.starting.has(stack.id)}
                error={installer.errors[stack.id] ?? null}
                installLabel={`Install (${decimalGb(stackBytes(stack))})`}
                quiet
                onInstall={() => void installer.install(stack)}
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
