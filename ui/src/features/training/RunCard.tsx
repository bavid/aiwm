import { useId, useState } from "react";
import { humanize } from "../../lib/errors";
import { useTrainingRun } from "../../lib/hooks";
import {
  cancelTrainingRun,
  deleteTrainingRun,
  pauseTrainingRun,
  resumeTrainingRun,
  trainingSampleUrl,
  type TrainingRun,
  type TrainingRunState,
} from "../../lib/ipc";

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

function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "—";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${String(s).padStart(2, "0")}s`;
  return `${s}s`;
}

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

type Props = {
  run: TrainingRun;
  profileLabel: string;
  /** Library name of the imported LoRA, once the run completed. */
  loraName: string | null;
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
  loraName,
  corePort,
  onChanged,
  onTestLora,
}: Props) {
  const logId = useId();
  const [expanded, setExpanded] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [purge, setPurge] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [brokenSamples, setBrokenSamples] = useState<ReadonlySet<number>>(() => new Set());

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

  return (
    <article className="card runcard">
      <header className="runcard__head">
        <h3>{run.name}</h3>
        <span className="runcard__profile">{profileLabel}</span>
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

      {run.state === "finishing" && (
        <p className="muted">
          Training is done — importing its result into the model library. This cannot be cancelled;
          it finishes on its own.
        </p>
      )}

      {run.error_text && <p className="runcard__err">{run.error_text}</p>}

      {samples.length > 0 && (
        <div className="runcard__samples">
          {samples.map((token, i) =>
            brokenSamples.has(i) || corePort === null ? (
              <div key={token} className="runcard__sample runcard__sample--missing">
                no preview yet
              </div>
            ) : (
              <img
                key={token}
                className="runcard__sample"
                src={trainingSampleUrl(corePort, run.id, i)}
                alt={`Preview ${i + 1} of ${run.name}`}
                width={120}
                height={120}
                onError={() => setBrokenSamples((cur) => new Set(cur).add(i))}
              />
            ),
          )}
        </div>
      )}

      <div className="runcard__actions">
        <button
          type="button"
          className="chip"
          disabled={busy || run.state !== "running"}
          onClick={() => void act(() => pauseTrainingRun(run.id))}
        >
          Pause
        </button>
        <button
          type="button"
          className="chip"
          disabled={busy || (run.state !== "paused" && run.state !== "interrupted")}
          onClick={() => void act(() => resumeTrainingRun(run.id))}
        >
          Resume
        </button>
        <button
          type="button"
          className="chip"
          disabled={busy || !CANCELLABLE.includes(run.state)}
          onClick={() => void act(() => cancelTrainingRun(run.id))}
        >
          Cancel
        </button>
        <button
          type="button"
          className="chip"
          disabled={busy || !DELETABLE.includes(run.state)}
          onClick={() => setConfirmDelete(true)}
        >
          Delete
        </button>
        {resultModelId && (
          <button
            type="button"
            className="chip"
            onClick={() => onTestLora(resultModelId, run.trigger_word)}
          >
            Test now
          </button>
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
          <label>
            <input type="checkbox" checked={purge} onChange={(e) => setPurge(e.target.checked)} />
            also delete its work folder (checkpoints and previews)
          </label>
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
