import { useEffect, useMemo, useRef, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import {
  agentSessionDetail,
  agentSessionMessage,
  agentSessionPermission,
  stopAgentSession,
  type AgentSessionDetail,
  type AgentSessionState,
  type PermissionDecision,
} from "../../lib/ipc";
import { groupEvents, type PermissionBlock } from "./transcript";
import { TranscriptBlock } from "./TranscriptView";

const POLL_MS = 700;
const TERMINAL: AgentSessionState[] = ["stopped", "failed"];

/** The running session: transcript, inline approvals, composer, Stop. Polls
 *  `agentSessionDetail` while the session is live. */
export function SessionPanel({
  sessionId,
  profileName,
  onClosed,
}: {
  sessionId: string | null;
  profileName: string | null;
  onClosed: () => void;
}) {
  const [detail, setDetail] = useState<AgentSessionDetail | null>(null);
  const [message, setMessage] = useState("");
  const [answered, setAnswered] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setDetail(null);
    setAnswered(new Set());
    setActionError(null);
    if (!sessionId) return;
    let alive = true;
    const tick = async () => {
      try {
        const d = await agentSessionDetail(sessionId);
        if (alive && d) setDetail(d);
      } catch {
        /* transient — next tick retries */
      }
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [sessionId]);

  const blocks = useMemo(() => groupEvents(detail?.events ?? []), [detail?.events]);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [blocks.length]);

  if (!sessionId) {
    return (
      <section className="card agents__session">
        <header className="card__head">
          <h2>Session</h2>
          <span className="card__sub">
            <HelpHint area="agents" setting="session" />
          </span>
        </header>
        <p className="muted">
          Pick a profile and hit <strong>New session</strong>. The transcript, tool calls and
          approval prompts show up here. <HelpHint area="agents" setting="approvals" />
        </p>
      </section>
    );
  }

  const state = detail?.session.state ?? "starting";
  const stoppable = !TERMINAL.includes(state);
  const terminal = TERMINAL.includes(state);
  const pendingPermission = blocks
    .filter((b): b is PermissionBlock => b.block === "permission")
    .find((b) => !answered.has(b.id));

  const send = async () => {
    const text = message.trim();
    if (!text || state !== "idle") return;
    setActionError(null);
    try {
      await agentSessionMessage(sessionId, text);
      setMessage("");
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  };

  const answer = async (requestId: string, decision: PermissionDecision) => {
    setActionError(null);
    setAnswered((s) => new Set(s).add(requestId));
    try {
      await agentSessionPermission(sessionId, requestId, decision);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
      setAnswered((s) => {
        const n = new Set(s);
        n.delete(requestId);
        return n;
      });
    }
  };

  const stop = async () => {
    try {
      await stopAgentSession(sessionId);
    } catch {
      /* best effort */
    }
  };

  return (
    <section className="card agents__session">
      <header className="card__head">
        <h2>Session</h2>
        <span className="card__sub">
          {profileName && <span className="numeric">{profileName} · </span>}
          <span className="state" data-state={state}>
            {state.replace("_", " ")}
          </span>{" "}
          <HelpHint area="agents" setting="approvals" />
        </span>
      </header>

      <div className="transcript" ref={logRef}>
        {blocks.length === 0 && <p className="muted">Starting the runtime…</p>}
        {blocks.map((b, i) => (
          <TranscriptBlock
            key={b.block === "tool" || b.block === "permission" ? b.id : `${b.block}-${i}`}
            block={b}
            answered={b.block === "permission" && answered.has(b.id)}
            onAnswer={answer}
          />
        ))}
        {state === "failed" && detail?.session.error_text && (
          <div className="tblock tblock--error">{detail.session.error_text}</div>
        )}
      </div>

      {actionError && <p className="agents__err">{actionError}</p>}

      {!terminal && (
        <form
          className="agents__composer"
          onSubmit={(e) => {
            e.preventDefault();
            send();
          }}
        >
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={2}
            spellCheck
            placeholder={
              pendingPermission
                ? "Answer the approval prompt above first."
                : state === "idle"
                  ? "Message the agent — Enter to send"
                  : `Agent is ${state.replace("_", " ")}…`
            }
            disabled={state !== "idle"}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
          />
          <div className="agents__composer-actions">
            <button type="submit" disabled={!message.trim() || state !== "idle"}>
              Send
            </button>
            <button type="button" className="agents__stop" onClick={stop} disabled={!stoppable}>
              Stop
            </button>
          </div>
        </form>
      )}

      {terminal && (
        <button type="button" className="agents__newbtn" onClick={onClosed}>
          Close transcript
        </button>
      )}
    </section>
  );
}
