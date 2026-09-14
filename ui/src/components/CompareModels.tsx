import { useEffect, useRef, useState } from "react";
import { jobDetail, submitJob, type Model } from "../lib/ipc";
import "./compare-models.css";

type Side = {
  jobId: string | null;
  modelId: string;
  answer: string;
  stats: string | null;
  state: "idle" | "running" | "blocked" | "completed" | "failed";
  error: string | null;
};

const emptySide = (modelId: string): Side => ({
  jobId: null,
  modelId,
  answer: "",
  stats: null,
  state: "idle",
  error: null,
});

function statsFrom(events: { message: string }[]): string | null {
  const e = events.find((x) => x.message.startsWith("answered"));
  const m = e?.message.match(/(\d+)\s+tokens,\s+([\d.]+)\s+tok\/s/);
  return m ? `${m[1]} tokens · ${m[2]} tok/s` : null;
}

/** Send the same prompt to two models at once and read the answers side by
 *  side -- an ad-hoc comparison, not a persisted chat: closing this loses
 *  the run, same as trying two tabs and comparing by eye, just without the
 *  eye strain. */
export function CompareModels({
  open,
  onClose,
  chatModels,
}: {
  open: boolean;
  onClose: () => void;
  chatModels: Model[];
}) {
  const [prompt, setPrompt] = useState("");
  const [left, setLeft] = useState<Side>(() => emptySide(""));
  const [right, setRight] = useState<Side>(() => emptySide(""));
  const [running, setRunning] = useState(false);
  // Stops the background poll loop once the dialog is closed -- the
  // component itself stays mounted (Chat renders it unconditionally), so
  // without this a closed comparison would keep polling forever.
  const stoppedRef = useRef(!open);
  useEffect(() => {
    stoppedRef.current = !open;
  }, [open]);

  // `chatModels` is still loading (or empty) at the moment this component
  // first mounts -- reset to real defaults every time the modal actually
  // opens, once real models are available, rather than a stale lazy-init
  // that ran before useModels() resolved.
  useEffect(() => {
    if (!open || chatModels.length === 0) return;
    setLeft(emptySide(chatModels[0].id));
    setRight(emptySide(chatModels[1]?.id ?? chatModels[0].id));
    setPrompt("");
    // Only re-run when the modal is (re)opened, not on every chatModels poll
    // tick -- that would reset an in-progress comparison out from under the
    // user every few seconds.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!open) return null;

  const byId = new Map(chatModels.map((m) => [m.id, m]));

  const runOne = async (modelId: string, text: string, set: (fn: (s: Side) => Side) => void) => {
    const model = byId.get(modelId);
    if (!model) return;
    const isColibri = model.format === "colibri";
    try {
      const job = await submitJob({
        job_type: isColibri ? "colibri" : "chat",
        model_id: model.id,
        runtime_id: isColibri ? "colibri" : "llamacpp",
        params: { prompt: text },
      });
      set((s) => ({ ...s, jobId: job.id, state: "running", answer: "", error: null }));
      const DONE = new Set(["completed", "failed", "cancelled"]);
      for (;;) {
        await new Promise((r) => setTimeout(r, 400));
        if (stoppedRef.current) return;
        const detail = await jobDetail(job.id);
        if (!detail) break;
        const { job: j, events } = detail;
        set((s) => ({
          ...s,
          answer: j.result ?? s.answer,
          stats: statsFrom(events) ?? s.stats,
          error: j.error_text,
        }));
        if (DONE.has(j.state)) {
          set((s) => ({ ...s, state: j.state === "completed" ? "completed" : "failed" }));
          break;
        }
        // `blocked` (not enough VRAM right now) isn't terminal -- the
        // scheduler re-checks every tick and can un-block on its own once
        // something else frees VRAM -- so keep polling, just show why it's
        // stuck instead of a silent "Thinking..." forever.
        set((s) => ({ ...s, state: j.state === "blocked" ? "blocked" : "running" }));
      }
    } catch (err) {
      set((s) => ({ ...s, state: "failed", error: err instanceof Error ? err.message : String(err) }));
    }
  };

  const run = async () => {
    const text = prompt.trim();
    if (!text || running) return;
    setRunning(true);
    try {
      await Promise.all([runOne(left.modelId, text, setLeft), runOne(right.modelId, text, setRight)]);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="cmdk__backdrop" onClick={onClose}>
      <div
        className="compare"
        role="dialog"
        aria-label="Compare models"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="compare__head">
          <h2>Compare models</h2>
          <button type="button" className="job-cancel" onClick={onClose}>
            Close
          </button>
        </div>

        <textarea
          className="compare__prompt"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          rows={2}
          placeholder="One prompt, sent to both models at once…"
        />

        <div className="compare__cols">
          <CompareSide side={left} models={chatModels} onModelChange={(id) => setLeft(emptySide(id))} />
          <CompareSide side={right} models={chatModels} onModelChange={(id) => setRight(emptySide(id))} />
        </div>

        <button type="button" className="compare__run" onClick={run} disabled={!prompt.trim() || running}>
          {running ? "Running both…" : "Run on both"}
        </button>
      </div>
    </div>
  );
}

function CompareSide({
  side,
  models,
  onModelChange,
}: {
  side: Side;
  models: Model[];
  onModelChange: (id: string) => void;
}) {
  return (
    <div className="compare__side">
      <select value={side.modelId} onChange={(e) => onModelChange(e.target.value)}>
        {models.map((m) => (
          <option key={m.id} value={m.id}>
            {m.name}
          </option>
        ))}
      </select>
      <div className="compare__answer" data-state={side.state}>
        {side.state === "idle" && <span className="muted">Waiting to run.</span>}
        {side.state === "running" && !side.answer && <span className="muted">Thinking…</span>}
        {side.answer}
        {side.state === "failed" && <span className="compare__err">{side.error ?? "failed"}</span>}
        {side.state === "blocked" && (
          <span className="compare__err">{side.error ?? "not enough VRAM free right now"}</span>
        )}
      </div>
      {side.stats && <div className="compare__stats numeric">{side.stats}</div>}
    </div>
  );
}
