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
import { approvalDomId, clockOf, SESSION_STATE } from "./labels";
import { groupEvents, type PermissionBlock } from "./transcript";
import { TranscriptBlock } from "./TranscriptView";

const POLL_MS = 700;
const TERMINAL: AgentSessionState[] = ["stopped", "failed"];

/** Moves focus to `el` only when nothing else has it — focus follows the
 *  handover ("your turn", "session over") without yanking the caret out of
 *  whatever the person was reading or pressing. */
function focusIfIdle(root: HTMLElement | null, el: HTMLElement | null) {
  if (!el) return;
  const active = document.activeElement;
  const parked = active == null || active === document.body;
  if (parked || (root != null && active instanceof Node && root.contains(active))) el.focus();
}

/** The running session: status, transcript, inline approvals, composer, Stop.
 *  Polls `agentSessionDetail` while the session is live. */
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
  const [decisions, setDecisions] = useState<Map<string, PermissionDecision>>(new Map());
  const [actionError, setActionError] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const prevState = useRef<AgentSessionState | null>(null);

  useEffect(() => {
    setDetail(null);
    setDecisions(new Map());
    setActionError(null);
    prevState.current = null;
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
  const state = detail?.session.state ?? "starting";

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [blocks.length]);

  // Focus follows the handover: the composer when the turn comes back, the
  // close button when the session is over.
  useEffect(() => {
    const prev = prevState.current;
    prevState.current = state;
    if (prev == null || prev === state) return;
    if (state === "idle") focusIfIdle(rootRef.current, composerRef.current);
    else if (TERMINAL.includes(state)) focusIfIdle(rootRef.current, closeRef.current);
  }, [state]);

  if (!sessionId) return <EmptySession />;

  const stoppable = !TERMINAL.includes(state);
  const terminal = TERMINAL.includes(state);
  const pending = blocks
    .filter((b): b is PermissionBlock => b.block === "permission")
    .find((b) => !decisions.has(b.id));

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
    setDecisions((m) => new Map(m).set(requestId, decision));
    try {
      await agentSessionPermission(sessionId, requestId, decision);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
      setDecisions((m) => {
        const n = new Map(m);
        n.delete(requestId);
        return n;
      });
    }
  };

  const stop = async () => {
    try {
      await stopAgentSession(sessionId);
    } catch {
      /* best effort — the poll reports the real state */
    }
  };

  const jumpToPrompt = () => {
    if (!pending) return;
    const el = document.getElementById(approvalDomId(pending.id));
    el?.scrollIntoView({ block: "nearest" });
    el?.focus();
  };

  return (
    <section className="card agents__session" ref={rootRef} aria-labelledby="agents-session-h">
      <header className="sess__head">
        <div className="sess__title">
          <h2 id="agents-session-h">Session</h2>
          {profileName && <span className="sess__profile">{profileName}</span>}
          {detail && (
            <span className="sess__since numeric">
              since {clockOf(detail.session.started_at)}
            </span>
          )}
          <HelpHint area="agents" setting="session" />
        </div>
        <SessionStatus state={state} />
        {pending && (
          <button type="button" className="sess__jump" onClick={jumpToPrompt}>
            Go to the approval prompt
          </button>
        )}
      </header>

      <div className="transcript" ref={logRef} role="log" aria-label="Agent transcript">
        {blocks.length === 0 && (
          <p className="muted">
            Starting the runtime and loading its model. The agent's messages, every tool call
            with its command and output, and every approval prompt appear here.
          </p>
        )}
        {blocks.map((b, i) => (
          <TranscriptBlock
            key={b.block === "tool" || b.block === "permission" ? b.id : `${b.block}-${i}`}
            block={b}
            decision={b.block === "permission" ? (decisions.get(b.id) ?? null) : null}
            onAnswer={answer}
          />
        ))}
        {state === "failed" && detail?.session.error_text && (
          <div className="tblock tblock--error">
            <span className="tblock__who">Runtime error</span>
            <p className="tblock__body">{detail.session.error_text}</p>
          </div>
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
          <label className="visually-hidden" htmlFor="agents-composer">
            Message the agent
          </label>
          <textarea
            id="agents-composer"
            ref={composerRef}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={2}
            spellCheck
            placeholder={
              pending
                ? "Answer the approval prompt first."
                : state === "idle"
                  ? "Tell the agent what to do — Enter sends, Shift+Enter for a new line"
                  : `${SESSION_STATE[state].label} — ${SESSION_STATE[state].hint}`
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
            <HelpHint area="agents" setting="approvals" />
            <button type="button" className="agents__stop" onClick={stop} disabled={!stoppable}>
              Stop session
            </button>
          </div>
        </form>
      )}

      {terminal && (
        <div className="sess__over">
          <p className="muted">
            The transcript stays until you close it; closing it frees the panel for the next
            session.
          </p>
          <button ref={closeRef} type="button" className="agents__newbtn" onClick={onClosed}>
            Close transcript
          </button>
        </div>
      )}
    </section>
  );
}

/** The state as a dot, a word and a sentence — announced when it changes on
 *  its own, and never carried by colour alone. */
function SessionStatus({ state }: { state: AgentSessionState }) {
  const copy = SESSION_STATE[state];
  return (
    <p className="sstatus" data-state={state} aria-live="polite">
      <span className="sstatus__dot" data-state={state} aria-hidden="true" />
      <strong className="sstatus__label">{copy.label}</strong>
      <span className="sstatus__hint">{copy.hint}</span>
    </p>
  );
}

/** No session yet: say what this panel is for and what to press, instead of
 *  an empty box. */
function EmptySession() {
  return (
    <section className="card agents__session agents__session--empty" aria-labelledby="agents-session-h">
      <header className="sess__head">
        <div className="sess__title">
          <h2 id="agents-session-h">Session</h2>
          <HelpHint area="agents" setting="session" />
        </div>
      </header>
      <div className="sess__placeholder">
        <p className="sess__placeholder-lead">
          Press <strong>Start session</strong> on a profile to bring an agent up here.
        </p>
        <ul className="sess__preview">
          <li>Its messages as it works, newest at the bottom.</li>
          <li>Every tool call with the exact command and its output.</li>
          <li>
            An approval prompt before any command runs or any file changes — the turn waits
            for your answer. <HelpHint area="agents" setting="approvals" />
          </li>
        </ul>
        <p className="muted">
          One session runs at a time, and the coding model is loaded at its full context
          rather than the 8,192-token chat default.
        </p>
      </div>
    </section>
  );
}
