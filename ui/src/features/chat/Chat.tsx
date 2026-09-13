import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ChatSessionSidebar } from "../../components/ChatSessionSidebar";
import { CompareModels } from "../../components/CompareModels";
import { useAbout, useDocuments, useJobs, useModels, useRuntimes } from "../../lib/hooks";
import {
  attachDocument,
  cancelJob,
  deleteDocument,
  deleteJob,
  jobDetail,
  jobOutputUrl,
  submitJob,
  type Job,
  type JobEvent,
  type JobState,
} from "../../lib/ipc";
import "./chat.css";

type TurnKind = "text" | "image" | "video";

type Turn = {
  jobId: string;
  kind: TurnKind;
  prompt: string;
  answer: string;
  state: JobState;
  model: string | null;
  stats: string | null;
  error: string | null;
};

const DONE: JobState[] = ["completed", "failed", "cancelled"];
/** Every job type this tab's history can include — "colibri" chats run on a
 *  different runtime than "chat" (llama.cpp), and "image"/"video" are here
 *  because `/image`/`/video` let you generate media without leaving the
 *  conversation — but all four are the same thread from the user's point
 *  of view. */
const CHAT_JOB_TYPES = ["chat", "colibri", "image", "video"];
const POLL_MS = 350;

/** `/image <prompt>` or `/video <prompt>` at the start of a message routes
 *  to that capability instead of the chat model -- reliable and explicit,
 *  rather than guessing intent from natural language. */
const MEDIA_COMMAND = /^\/(image|video)\s+(.+)$/is;

function kindOf(jobType: string): TurnKind {
  return jobType === "image" || jobType === "video" ? jobType : "text";
}

/** Sensible defaults matching the Image/Video studios' own initial state --
 *  a `/image`/`/video` command from Chat skips the full form, so it needs
 *  something reasonable to submit. */
function defaultMediaParams(kind: "image" | "video", prompt: string): Record<string, unknown> {
  if (kind === "image") {
    return { prompt, negative: "", width: 1024, height: 1024, steps: 25, cfg: 7 };
  }
  return { prompt, negative: "", width: 832, height: 480, length: 81, fps: 24, steps: 30, cfg: 5 };
}

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

function promptOf(job: Job): string {
  const p = job.params;
  return p && typeof p === "object" && typeof (p as { prompt?: unknown }).prompt === "string"
    ? (p as { prompt: string }).prompt
    : "";
}

export function Chat() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [modelId, setModelId] = useState("auto");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [prompt, setPrompt] = useState("");
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [compareOpen, setCompareOpen] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);
  // Which session's history `turns` currently reflects -- `undefined` means
  // "nothing loaded yet". Re-derive from the jobs table on mount and whenever
  // the user switches sessions; otherwise leave `turns` alone so an optimistic
  // send() (or a live jobDetail poll) isn't clobbered by a lagging jobs poll.
  const hydratedFor = useRef<string | null | undefined>(undefined);

  const { data: models } = useModels();
  const { data: runtimes } = useRuntimes();
  const { data: jobs } = useJobs();
  const about = useAbout();
  const chatModels = useMemo(
    () => (models ?? []).filter((m) => m.roles.includes("chat")),
    [models],
  );
  const hasChatModel = chatModels.length > 0;
  const llama = (runtimes ?? []).find((r) => r.id === "llamacpp");
  const llamaReady = !llama || !(llama.detail ?? "").includes("not installed");
  const modelNames = useMemo(
    () => new Map((models ?? []).map((m) => [m.id, m.name])),
    [models],
  );
  const modelById = useMemo(
    () => new Map((models ?? []).map((m) => [m.id, m])),
    [models],
  );

  // Past turns live in the jobs table already (job_type "chat") -- pull them in
  // on mount and on every session switch so the conversation survives
  // switching tabs, switching sessions, or restarting the app, instead of
  // vanishing with this component's local state.
  useEffect(() => {
    if (!jobs || hydratedFor.current === sessionId) return;
    hydratedFor.current = sessionId;
    const history = jobs
      .filter((j) => {
        if (j.session_id !== sessionId) return false;
        // "Ungrouped" (no session) is a single shared bucket across every tab
        // -- an image generated on the Image tab with no session picked has
        // the exact same `session_id: null` as an ungrouped chat. Only a real
        // session id is unique enough to prove an image/video job actually
        // came from this chat's own /image · /video command; ungrouped, only
        // count real chat turns.
        if (sessionId === null) return j.job_type === "chat" || j.job_type === "colibri";
        return CHAT_JOB_TYPES.includes(j.job_type);
      })
      .slice()
      .sort((a, b) => a.created_at.localeCompare(b.created_at))
      .map(
        (j): Turn => ({
          jobId: j.id,
          kind: kindOf(j.job_type),
          prompt: promptOf(j),
          answer: j.result ?? "",
          state: j.state,
          model: j.model_id ? (modelNames.get(j.model_id) ?? j.model_id) : null,
          stats: null,
          error: j.error_text,
        }),
      );
    setTurns(history);
  }, [jobs, modelNames, sessionId]);

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
                model:
                  modelFromEvents(events) ??
                  (job.model_id ? modelNames.get(job.model_id) : undefined) ??
                  t.model,
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
  }, [pendingId, modelNames]);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns]);

  // A `blocked` job (not enough VRAM right now) isn't actively running -- it's
  // just waiting for room, and may sit there indefinitely if none frees up.
  // Don't lock the composer forever: let the user try again instead of being
  // stuck until they cancel or switch tabs.
  const stuck = turns.find((t) => t.jobId === pendingId)?.state === "blocked";

  // `turns` is only re-derived from the jobs poll on mount/session-switch (see
  // the hydration effect above), so a delete must splice local state directly
  // rather than waiting for the next poll tick to notice the job is gone.
  const handleDeleteTurn = async (jobId: string) => {
    try {
      await deleteJob(jobId);
      setTurns((ts) => ts.filter((t) => t.jobId !== jobId));
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  const send = async () => {
    const text = prompt.trim();
    if (!text || (pendingId && !stuck)) return;
    setSendError(null);

    const media = text.match(MEDIA_COMMAND);

    try {
      let job: Job;
      let turnModel: string | null;
      if (media) {
        const kind = media[1] as "image" | "video";
        const mediaPrompt = media[2].trim();
        job = await submitJob({
          job_type: kind,
          params: defaultMediaParams(kind, mediaPrompt),
          session_id: sessionId ?? undefined,
        });
        turnModel = null; // Auto-picked; the poll below fills in the real name once it lands.
        setTurns((ts) => [
          ...ts,
          {
            jobId: job.id,
            kind,
            prompt: mediaPrompt,
            answer: "",
            state: job.state,
            model: turnModel,
            stats: null,
            error: null,
          },
        ]);
      } else {
        const picked = modelId === "auto" ? null : modelById.get(modelId);
        // "Auto" always resolves onto llama.cpp; an explicit pick routes to
        // whichever runtime actually serves that model's format (Colibri
        // models are never part of the Auto pool — see `for_role_with_benchmark`).
        const isColibri = picked?.format === "colibri";
        job = await submitJob({
          job_type: isColibri ? "colibri" : "chat",
          model_id: picked ? picked.id : undefined,
          runtime_id: picked ? (isColibri ? "colibri" : "llamacpp") : undefined,
          params: { prompt: text },
          session_id: sessionId ?? undefined,
        });
        setTurns((ts) => [
          ...ts,
          {
            jobId: job.id,
            kind: "text",
            prompt: text,
            answer: "",
            state: job.state,
            model: picked?.name ?? null,
            stats: null,
            error: null,
          },
        ]);
      }
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
    <div className="chat-page">
      <ChatSessionSidebar activeId={sessionId} onChange={setSessionId} />
      <div className="chat">
        <div className="chat__head">
          <select
            className="chat__model"
            value={modelId}
            onChange={(e) => setModelId(e.target.value)}
            title="Which model answers"
          >
            <option value="auto">Auto (most-recently-used)</option>
            {chatModels.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
          {chatModels.length >= 2 && (
            <button
              type="button"
              className="chat__compare"
              onClick={() => setCompareOpen(true)}
              title="Send one prompt to two models at once"
            >
              Compare models
            </button>
          )}
        </div>
        <CompareModels open={compareOpen} onClose={() => setCompareOpen(false)} chatModels={chatModels} />
        <DocumentsBar sessionId={sessionId} />
        <div className="chat__log" ref={logRef}>
          {turns.length === 0 && (
            <div className="chat__empty">
              <p>Ask anything. A chat model is picked automatically.</p>
              <p className="muted">
                Try <code>/image a bay at dawn</code> or <code>/video a paper boat in the rain</code>{" "}
                to generate media without leaving the conversation.
              </p>
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
            <ChatTurn
              key={t.jobId}
              turn={t}
              port={about?.core_api_port ?? null}
              onCancel={() => cancelJob(t.jobId)}
              onDelete={() => handleDeleteTurn(t.jobId)}
            />
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
              pendingId && !stuck
                ? "Waiting for the answer…"
                : "Message, or /image · /video a prompt — Enter to send, Shift+Enter for a newline"
            }
          />
          <button type="submit" disabled={!prompt.trim() || (!!pendingId && !stuck)}>
            {pendingId && !stuck ? "…" : "Send"}
          </button>
        </form>
        {sendError && <p className="chat__err">{sendError}</p>}
      </div>
    </div>
  );
}

/** Documents attached to the active session ground the model's answers via
 *  lexical (keyword) search — no embedding model, see the scoping notes.
 *  Attaching is session-scoped: switch sessions and this bar switches with
 *  it, same as the rest of the conversation. */
function DocumentsBar({ sessionId }: { sessionId: string | null }) {
  const { data: documents } = useDocuments(sessionId);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!sessionId) return null;

  const attach = async () => {
    setError(null);
    let picked: string | null;
    try {
      const result = await open({
        multiple: false,
        filters: [{ name: "Text documents", extensions: ["txt", "md"] }],
      });
      picked = typeof result === "string" ? result : null;
    } catch {
      return; // Not running inside Tauri (e.g. the browser dev preview) -- no-op.
    }
    if (!picked) return;

    setBusy(true);
    try {
      await attachDocument(sessionId, picked);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: string) => {
    setError(null);
    try {
      await deleteDocument(id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const docs = documents ?? [];

  return (
    <div className="chat__docs">
      {docs.map((d) => (
        <span key={d.id} className="chat__doc-chip">
          {d.name}
          <button
            type="button"
            className="chat__doc-remove"
            onClick={() => remove(d.id)}
            aria-label={`Remove ${d.name}`}
          >
            ×
          </button>
        </span>
      ))}
      <button type="button" className="chat__doc-attach" onClick={attach} disabled={busy}>
        {busy ? "Adding…" : "+ Attach document"}
      </button>
      {error && <span className="chat__doc-err">{error}</span>}
    </div>
  );
}

function ChatTurn({
  turn,
  port,
  onCancel,
  onDelete,
}: {
  turn: Turn;
  port: number | null;
  onCancel: () => void;
  onDelete: () => void;
}) {
  const running = !DONE.includes(turn.state);
  const waiting = running && !turn.answer && turn.state !== "blocked";
  const isMedia = turn.kind !== "text";

  return (
    <div className="turn">
      <div className="turn__you">
        {isMedia && <span className="turn__cmd">/{turn.kind}</span>} {turn.prompt}
      </div>
      <div className="turn__answer" data-state={turn.state}>
        {isMedia ? (
          <MediaAnswer turn={turn} port={port} />
        ) : (
          <>
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
          </>
        )}
        {turn.state === "failed" && (
          <span className="turn__err">{turn.error ?? "failed"}</span>
        )}
        {turn.state === "blocked" && (
          <span className="turn__err">{turn.error ?? "not enough VRAM free right now"}</span>
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
        {!running && (
          <button type="button" className="turn__cancel" onClick={onDelete} title="Delete this message">
            Delete
          </button>
        )}
      </div>
    </div>
  );
}

function MediaAnswer({ turn, port }: { turn: Turn; port: number | null }) {
  const running = !DONE.includes(turn.state);

  if (turn.state === "completed" && port != null) {
    return turn.kind === "video" ? (
      <video className="turn__media" src={jobOutputUrl(port, turn.jobId)} controls preload="metadata" />
    ) : (
      <img className="turn__media" src={jobOutputUrl(port, turn.jobId)} alt={turn.prompt} />
    );
  }
  if (running) {
    return <span className="turn__wait">{turn.state === "blocked" ? "" : `${turn.state}…`}</span>;
  }
  return null;
}
