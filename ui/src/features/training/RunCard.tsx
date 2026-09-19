import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { humanize } from "../../lib/errors";
import { useTrainingRun } from "../../lib/hooks";
import {
  cancelTrainingRun,
  deleteTrainingRun,
  pauseTrainingRun,
  resumeTrainingRun,
  type TrainingPresetValues,
  type TrainingRun,
  type TrainingRunState,
} from "../../lib/ipc";
import { formatDuration, formatWhen } from "./format";
import { effectiveHyperparams, parseHyperparamsJson } from "./hyperparams";
import { SampleStrip } from "./SampleStrip";

/** What each lifecycle state reads as. `interrupted` is the one that asks a
 *  question rather than reporting a fact — the run is intact and resumable. */
const STATE_LABEL: Record<TrainingRunState, string> = {
  preparing: "preparing",
  running: "running",
  paused: "paused",
  interrupted: "interrupted — resume?",
  resuming: "resuming",
  finishing: "finishing",
  completed: "completed",
  failed: "failed",
  cancelled: "cancelled",
};

/** States where the trainer process is (meant to be) alive, so the card keeps
 *  polling the run detail for fresh samples without being expanded. */
const LIVE: TrainingRunState[] = ["preparing", "running", "resuming", "finishing"];
const DELETABLE: TrainingRunState[] = ["completed", "failed", "cancelled"];

/** `finishing` is deliberately absent: the state machine only lets it go to
 *  `completed` or `failed`, so a Cancel offered there would be a button that
 *  cannot do what it says. The card explains the wait instead. */
const CANCELLABLE: TrainingRunState[] = [
  "preparing",
  "running",
  "paused",
  "interrupted",
  "resuming",
];

/** How long a run has to have been going before its step rate says anything. */
const MIN_ETA_ELAPSED_SECS = 20;

/** Steps done so far set the pace for the ones left. Deliberately naive: the
 *  trainer's own rate wanders, and a wrong-by-a-minute estimate still answers
 *  the only question being asked ("coffee, or overnight?"). */
function eta(run: TrainingRun): string | null {
  if (!run.started_at || run.step <= 0 || run.step >= run.total_steps) return null;
  const startedMs = Date.parse(run.started_at);
  if (!Number.isFinite(startedMs)) return null;
  const elapsed = (Date.now() - startedMs) / 1000;
  // The first seconds of a run carry no usable rate (the trainer is still
  // loading weights), and dividing by them produces an absurd estimate.
  if (elapsed < MIN_ETA_ELAPSED_SECS) return null;
  return formatDuration((elapsed / run.step) * (run.total_steps - run.step));
}

/** `rank 16 · lr 0.0001 · 1024px` from what the run actually trained with;
 *  a value nobody knows is left out rather than shown as a dash. */
function hyperparamLine(run: TrainingRun, preset: TrainingPresetValues | null): string {
  const hp = effectiveHyperparams(parseHyperparamsJson(run.hyperparams_json), preset);
  const parts = [
    hp.rank !== null ? `rank ${hp.rank}` : null,
    hp.lr !== null ? `lr ${hp.lr}` : null,
    hp.resolution !== null ? `${hp.resolution}px` : null,
  ];
  return parts.filter((p) => p !== null).join(" · ");
}

type Props = {
  run: TrainingRun;
  profileLabel: string;
  /** The run's preset as the profile defines it, for the values the run did
   *  not override; `null` while profiles load. */
  presetValues: TrainingPresetValues | null;
  /** The dataset the run trained on; `null` when its row is gone. */
  datasetName: string | null;
  /** Library name of the imported LoRA, once the run completed. */
  loraName: string | null;
  /** Library name of the LoRA this run continued from, when it started from
   *  one and that LoRA is still in the library. */
  initLoraName: string | null;
  /** `AboutInfo.core_api_port` — where the sample images are served from. */
  corePort: number | null;
  onChanged: () => void;
  onTestLora: (loraModelId: string, prompt: string) => void;
};

/** One run in the history: progress, the numbers worth watching, its preview
 *  images, an expandable log tail, and the lifecycle buttons. */
export function RunCard({
  run,
  profileLabel,
  presetValues,
  datasetName,
  loraName,
  initLoraName,
  corePort,
  onChanged,
  onTestLora,
}: Props) {
  const logId = useId();
  /** One id per action button, so its `?` hint can describe it. */
  const base = useId();
  const id = (action: string) => `${base}-${action}`;
  const [expanded, setExpanded] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [purge, setPurge] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isLive = LIVE.includes(run.state);
  const { data: detail } = useTrainingRun(expanded || isLive ? run.id : null);
  const samples = detail?.latest_samples ?? [];

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      onChanged();
    } catch (e) {
      setError(humanize(e));
    } finally {
      setBusy(false);
    }
  };

  const percent = run.total_steps > 0 ? Math.min(100, (run.step / run.total_steps) * 100) : 0;
  const remaining = eta(run);
  // Bound once so the "Test now" handler closes over a narrowed `string`
  // instead of the nullable field.
  const resultModelId = run.result_model_id;
  // The detail's name is authoritative once loaded; the list-derived one
  // covers the collapsed card without a second poll.
  const continuedFrom = detail?.init_lora_name ?? initLoraName;
  const hyperparams = hyperparamLine(run, presetValues);

  return (
    <article className="card runcard">
      <header className="runcard__head">
        <h3>{run.name}</h3>
        <span className="runcard__profile">{profileLabel}</span>
        {run.init_lora_model_id && (
          <span className="runcard__lineage">
            Continued from {continuedFrom ?? "a LoRA no longer in the library"}
          </span>
        )}
        <span className="runcard__chip" data-state={run.state}>
          {STATE_LABEL[run.state]}
        </span>
      </header>

      <div
        className="runcard__bar"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={run.total_steps}
        aria-valuenow={run.step}
        aria-label={`${run.name} progress`}
      >
        <div className="runcard__fill" style={{ width: `${percent}%` }} />
      </div>

      <div className="runcard__meta numeric">
        <span>
          step {run.step} / {run.total_steps}
        </span>
        <span>loss {run.last_loss === null ? "—" : run.last_loss.toFixed(4)}</span>
        <span>trigger {run.trigger_word || "—"}</span>
        {remaining && <span>about {remaining} left</span>}
        {resultModelId && (
          <span className="runcard__badge">LoRA in library{loraName ? `: ${loraName}` : ""}</span>
        )}
      </div>

      <dl className="runcard__facts numeric">
        <div>
          <dt>Dataset</dt>
          <dd>{datasetName ?? "—"}</dd>
        </div>
        <div>
          <dt>Preset</dt>
          <dd>
            {run.preset}
            {hyperparams && ` · ${hyperparams}`}
          </dd>
        </div>
        <div>
          <dt>Started</dt>
          <dd>{formatWhen(run.started_at)}</dd>
        </div>
        <div>
          <dt>Finished</dt>
          <dd>{formatWhen(run.finished_at)}</dd>
        </div>
      </dl>

      {run.state === "finishing" && (
        <p className="muted">
          Training is done — importing its result into the model library. This cannot be cancelled;
          it finishes on its own.
        </p>
      )}

      {run.error_text && <p className="runcard__err">{run.error_text}</p>}

      <SampleStrip runId={run.id} runName={run.name} tokens={samples} corePort={corePort} />

      <div className="runcard__actions">
        <button
          id={id("pause")}
          type="button"
          className="chip"
          disabled={busy || run.state !== "running"}
          onClick={() => void act(() => pauseTrainingRun(run.id))}
        >
          Pause
        </button>
        <HelpHint area="training" setting="pause" describes={id("pause")} />
        <button
          id={id("resume")}
          type="button"
          className="chip"
          disabled={busy || (run.state !== "paused" && run.state !== "interrupted")}
          onClick={() => void act(() => resumeTrainingRun(run.id))}
        >
          Resume
        </button>
        <HelpHint area="training" setting="resume" describes={id("resume")} />
        <button
          id={id("cancel")}
          type="button"
          className="chip"
          disabled={busy || !CANCELLABLE.includes(run.state)}
          onClick={() => void act(() => cancelTrainingRun(run.id))}
        >
          Cancel
        </button>
        <HelpHint area="training" setting="cancel" describes={id("cancel")} />
        <button
          id={id("delete")}
          type="button"
          className="chip"
          disabled={busy || !DELETABLE.includes(run.state)}
          onClick={() => setConfirmDelete(true)}
        >
          Delete
        </button>
        <HelpHint area="training" setting="delete-run" describes={id("delete")} />
        {resultModelId && (
          <>
            <button
              id={id("test")}
              type="button"
              className="chip"
              onClick={() => onTestLora(resultModelId, run.trigger_word)}
            >
              Test now
            </button>
            <HelpHint area="training" setting="test-now" describes={id("test")} />
          </>
        )}
        <button
          type="button"
          className="chip runcard__spacer"
          aria-expanded={expanded}
          aria-controls={logId}
          onClick={() => setExpanded((v) => !v)}
        >
          Log
        </button>
      </div>

      {confirmDelete && (
        <div className="runcard__confirm">
          <span>Delete “{run.name}” from the history?</span>
          <span className="runcard__purge">
            <input
              id={id("purge")}
              type="checkbox"
              checked={purge}
              onChange={(e) => setPurge(e.target.checked)}
            />
            <label htmlFor={id("purge")}>
              also delete its work folder (checkpoints and previews)
            </label>
            <HelpHint area="training" setting="purge-work-folder" describes={id("purge")} />
          </span>
          <button
            type="button"
            className="chip"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                await deleteTrainingRun(run.id, purge);
                setConfirmDelete(false);
              })
            }
          >
            Delete
          </button>
          <button type="button" className="chip" onClick={() => setConfirmDelete(false)}>
            Keep
          </button>
        </div>
      )}

      {/* Present even while collapsed so `aria-controls` above always resolves
          to a real element; `hidden` keeps it out of the tree either way. */}
      <div id={logId} hidden={!expanded}>
        <pre className="runcard__log">
          {detail?.log_tail.length ? detail.log_tail.join("\n") : "No log lines yet."}
        </pre>
        {detail && <p className="muted">{detail.work_dir}</p>}
      </div>

      {error && <p className="runcard__err">{error}</p>}
    </article>
  );
}
