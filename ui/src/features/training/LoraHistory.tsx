import { useEffect, useId, useRef } from "react";
import { HelpHint } from "../../components/HelpHint";
import type { LineageRun, LoraLineage, LoraSummary, TrainingProfile } from "../../lib/ipc";
import { formatCount, formatDuration, formatWhen } from "./format";
import { effectiveHyperparams } from "./hyperparams";
import { SampleStrip } from "./SampleStrip";

type Props = {
  lora: LoraSummary;
  /** `null` until the lineage poll answers. */
  lineage: LoraLineage | null;
  error: string | null;
  profiles: TrainingProfile[];
  /** `AboutInfo.core_api_port` — where the sample images are served from. */
  corePort: number | null;
  onTestLora: (loraModelId: string, prompt: string) => void;
  onContinue: () => void;
  onClose: () => void;
};

/** One contributing run, oldest first in the panel: what it was fed, what it
 *  was told, how it went, and what it produced. */
function HistoryRun({
  run,
  index,
  profiles,
  corePort,
}: {
  run: LineageRun;
  index: number;
  profiles: TrainingProfile[];
  corePort: number | null;
}) {
  const profile = profiles.find((p) => p.family === run.profile_family) ?? null;
  const hp = effectiveHyperparams(run.hyperparams, profile?.presets[run.preset] ?? null);
  const settings = [
    hp.rank !== null ? `rank ${hp.rank}` : null,
    hp.lr !== null ? `lr ${hp.lr}` : null,
    hp.resolution !== null ? `${hp.resolution}px` : null,
    `${run.step.toLocaleString()} / ${run.total_steps.toLocaleString()} steps`,
  ]
    .filter((s) => s !== null)
    .join(" · ");

  return (
    <li className="lorahistory__run">
      <header className="lorahistory__runhead">
        <span className="lorahistory__index numeric" aria-hidden="true">
          {index + 1}
        </span>
        <h5>{run.name}</h5>
        <span className="runcard__chip" data-state={run.state}>
          {run.state}
        </span>
      </header>

      <dl className="lorahistory__facts numeric">
        <div>
          <dt>Dataset</dt>
          <dd>
            {run.dataset ? (
              <>
                {run.dataset.name}
                <span className="lorahistory__path">{run.dataset.source_root}</span>
              </>
            ) : (
              "deleted since"
            )}
          </dd>
        </div>
        <div>
          <dt>Images</dt>
          <dd>{formatCount(run.image_count)}</dd>
        </div>
        <div>
          <dt>Captioner</dt>
          <dd>{run.dataset?.captioner ?? "—"}</dd>
        </div>
        <div>
          <dt>Trigger</dt>
          <dd>{run.trigger_word || "—"}</dd>
        </div>
        <div>
          <dt>Profile</dt>
          <dd>{profile?.label ?? run.profile_family}</dd>
        </div>
        <div>
          <dt>Preset</dt>
          <dd>
            {run.preset} · {settings}
          </dd>
        </div>
        <div>
          <dt>Duration</dt>
          <dd>
            {formatDuration(run.duration_secs)}
            <span className="lorahistory__path">
              {formatWhen(run.started_at)} → {formatWhen(run.finished_at)}
            </span>
          </dd>
        </div>
        <div>
          <dt>Result</dt>
          <dd>
            {run.result_model_name ?? (run.result_model_id ? "LoRA no longer in the library" : "—")}
            {run.init_lora_model_id && (
              <span className="lorahistory__path">
                continued from {run.init_lora_name ?? "a LoRA no longer in the library"}
              </span>
            )}
          </dd>
        </div>
      </dl>

      <SampleStrip runId={run.run_id} runName={run.name} tokens={run.samples} corePort={corePort} />
    </li>
  );
}

/** The history panel under "Your LoRAs": the selected LoRA's runs oldest
 *  first, plus the two things one does with a LoRA — try it, or train it on. */
export function LoraHistory({
  lora,
  lineage,
  error,
  profiles,
  corePort,
  onTestLora,
  onContinue,
  onClose,
}: Props) {
  const headingRef = useRef<HTMLHeadingElement>(null);
  const continueId = useId();

  // Selecting a row from the table above moves the reader to the panel it
  // opened; re-selecting another LoRA moves it again.
  useEffect(() => {
    headingRef.current?.focus();
  }, [lora.model_id]);

  const runs = lineage?.runs ?? [];
  const lastRun = runs.length > 0 ? runs[runs.length - 1] : null;
  const triggerWord = lastRun?.trigger_word ?? "";

  return (
    <section className="card lorahistory" aria-labelledby="lorahistory-heading">
      <header className="lorahistory__head">
        <div className="lorahistory__summary">
          <div className="training__headrow">
            <h4 id="lorahistory-heading" ref={headingRef} tabIndex={-1}>
              {lora.name}
            </h4>
            <HelpHint area="training" setting="lora-history" />
          </div>
          <p className="muted">
            {lora.trained
              ? `${lora.runs} ${lora.runs === 1 ? "run" : "runs"} · ${lora.total_steps.toLocaleString()} steps · ${formatCount(lora.total_images)} images`
              : "Imported by hand — no training history is known here."}
            {triggerWord && ` · trigger ${triggerWord}`}
          </p>
        </div>
        <div className="lorahistory__actions">
          <button
            type="button"
            className="chip"
            onClick={() => onTestLora(lora.model_id, triggerWord)}
          >
            Test in Image tab
          </button>
          <button
            id={continueId}
            type="button"
            className="chip lorahistory__continue"
            onClick={onContinue}
          >
            Continue with another dataset
          </button>
          <HelpHint area="training" setting="continue-training" describes={continueId} />
          <button type="button" className="chip" onClick={onClose}>
            Close
          </button>
        </div>
      </header>

      {error && <p className="training__err">{error}</p>}

      {lineage === null && !error ? (
        <p className="muted">Loading history…</p>
      ) : runs.length === 0 ? (
        <p className="muted">
          No run of this app produced this LoRA. Continue it on a dataset and the new version
          will list that run here.
        </p>
      ) : (
        <ol className="lorahistory__runs">
          {runs.map((run, i) => (
            <HistoryRun key={run.run_id} run={run} index={i} profiles={profiles} corePort={corePort} />
          ))}
        </ol>
      )}
    </section>
  );
}
