import { useEffect, useRef, useState } from "react";
import { useModels, useRuntimes } from "../../lib/hooks";
import {
  cancelJob,
  jobDetail,
  submitJob,
  type JobEvent,
  type JobState,
} from "../../lib/ipc";
import "./chat.css";

type Turn = {
  jobId: string;
  prompt: string;
  answer: string;
  state: JobState;
  model: string | null;
  stats: string | null;
  error: string | null;
};

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 350;

function modelFromEvents(events: JobEvent[]): string | null {
  const e = events.find((x) => x.message.startsWith("auto-selected model"));
  const m = e?.message.match(/model\s+["“](.+)["”]/);
  return m ? m[1] : null;
}

function statsFromEvents(events: JobEvent[]): string | null {
  const e = events.find((x) => x.message.startsWith("answered"));
  const m = e?.message.match(/(\d+)\s+tokens,\s+([\d.]+)\s+tok\/s/);
  return m ? `${m[1]} tokens · ${m[2]} tok/s` : null;
}

export function Chat() {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [prompt, setPrompt] = useState("");
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement>(null);

  const { data: models } = useModels();
  const { data: runtimes } = useRuntimes();
  const hasChatModel = (models ?? []).some((m) => m.roles.includes("chat"));
  const llama = (runtimes ?? []).find((r) => r.id === "llamacpp");
  const llamaReady = !llama || !(llama.detail ?? "").includes("not installed");

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    const tick = async () => {
      let detail;
      try {
        detail = await jobDetail(pendingId);
      } catch {
        return;
      }
      if (!alive || !detail) return;
      const { job, events } = detail;
      setTurns((ts) =>
        ts.map((t) =>
          t.jobId === pendingId
            ? {
                ...t,
                answer: job.result ?? t.answer,
                state: job.state,
                model: modelFromEvents(events) ?? t.model,
                stats: statsFromEvents(events) ?? t.stats,
                error: job.error_text,
              }
            : t,
        ),
      );
      if (DONE.includes(job.state)) setPendingId(null);
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId]);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns]);

  const send = async () => {
    const text = prompt.trim();
    if (!text || pendingId) return;
    setSendError(null);
    try {
      const job = await submitJob({ job_type: "chat", params: { prompt: text } });
      setTurns((ts) => [
        ...ts,
        {
          jobId: job.id,
          prompt: text,
          answer: "",
          state: job.state,
          model: null,
          stats: null,
          error: null,
        },
      ]);
      setPrompt("");
      setPendingId(job.id);
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  return (
    <div className="chat">
      <div className="chat__log" ref={logRef}>
        {turns.length === 0 && (
          <div className="chat__empty">
            <p>Ask anything. A chat model is picked automatically.</p>
            {!llamaReady && (
              <p className="muted">llama.cpp is not set up yet — open Diagnostics to install it.</p>
            )}
            {llamaReady && models && !hasChatModel && (
              <p className="muted">
                No model has the “chat” role — import a .gguf on the Models tab.
              </p>
            )}
          </div>
        )}
        {turns.map((t) => (
          <ChatTurn key={t.jobId} turn={t} onCancel={() => cancelJob(t.jobId)} />
        ))}
      </div>

      <form
        className="chat__composer"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={onKeyDown}
          rows={2}
          spellCheck
          placeholder={
            pendingId ? "Waiting for the answer…" : "Message — Enter to send, Shift+Enter for a newline"
          }
        />
        <button type="submit" disabled={!prompt.trim() || !!pendingId}>
          {pendingId ? "…" : "Send"}
        </button>
      </form>
      {sendError && <p className="chat__err">{sendError}</p>}
    </div>
  );
}

function ChatTurn({ turn, onCancel }: { turn: Turn; onCancel: () => void }) {
  const running = !DONE.includes(turn.state);
  const waiting = running && !turn.answer;

  return (
    <div className="turn">
      <div className="turn__you">{turn.prompt}</div>
      <div className="turn__answer" data-state={turn.state}>
        {waiting && <span className="turn__wait">{turn.state}…</span>}
        {turn.answer && (
          <>
            {turn.answer}
            {running && (
              <span className="turn__caret" aria-hidden="true">
                ▍
              </span>
            )}
          </>
        )}
        {turn.state === "failed" && (
          <span className="turn__err">{turn.error ?? "failed"}</span>
        )}
        {turn.state === "cancelled" && !turn.answer && <span className="muted">cancelled</span>}
      </div>
      <div className="turn__meta">
        {turn.model && <span>{turn.model}</span>}
        {turn.stats && <span>· {turn.stats}</span>}
        {turn.state === "cancelled" && turn.answer && <span>· cancelled</span>}
        {running && (
          <button type="button" className="turn__cancel" onClick={onCancel}>
            Stop
          </button>
        )}
      </div>
    </div>
  );
}
