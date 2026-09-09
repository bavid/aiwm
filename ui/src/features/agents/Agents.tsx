import { useEffect, useMemo, useRef, useState } from "react";
import { useAgents, useModels } from "../../lib/hooks";
import {
  agentSessionDetail,
  agentSessionMessage,
  agentSessionPermission,
  createAgent,
  deleteAgent,
  openAgentSession,
  stopAgentSession,
  type Agent,
  type AgentEventPayload,
  type AgentSessionDetail,
  type AgentSessionState,
  type PermissionDecision,
  type ToolStatus,
} from "../../lib/ipc";
import "./agents.css";

const POLL_MS = 700;
const TERMINAL: AgentSessionState[] = ["stopped", "failed"];

export function AgentsWorkbench() {
  const { data: profiles, refetch } = useAgents();
  const { data: models } = useModels();
  const codingModels = (models ?? []).filter((m) => m.roles.includes("coding"));

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [activeProfile, setActiveProfile] = useState<Agent | null>(null);
  const [startError, setStartError] = useState<string | null>(null);

  const start = async (profile: Agent, firstMessage?: string) => {
    setStartError(null);
    try {
      const session = await openAgentSession(profile.id, firstMessage);
      setActiveProfile(profile);
      setSessionId(session.id);
    } catch (err) {
      setStartError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="agents">
      <section className="card agents__profiles">
        <header className="card__head">
          <h2>Agent profiles</h2>
          <span className="card__sub numeric">{profiles?.length ?? 0}</span>
        </header>

        {profiles && profiles.length === 0 && (
          <p className="muted">
            No profiles yet. A profile binds an agent runtime to a workspace folder and a
            coding model.
          </p>
        )}
        <ul className="prof-list">
          {(profiles ?? []).map((p) => (
            <ProfileRow
              key={p.id}
              profile={p}
              modelName={p.model_id ? nameFor(codingModels, p.model_id) : null}
              disabled={sessionId != null}
              onStart={() => start(p)}
              onDelete={async () => {
                await deleteAgent(p.id);
                if (activeProfile?.id === p.id) setSessionId(null);
                refetch();
              }}
            />
          ))}
        </ul>

        <NewProfileForm codingModels={codingModels} onCreated={refetch} />
        {startError && <p className="agents__err">{startError}</p>}
      </section>

      <SessionPanel
        sessionId={sessionId}
        profileName={activeProfile?.name ?? null}
        onClosed={() => setSessionId(null)}
      />
    </div>
  );
}

function nameFor(models: { id: string; name: string }[], id: string): string {
  return models.find((m) => m.id === id)?.name ?? id;
}

function ProfileRow({
  profile,
  modelName,
  disabled,
  onStart,
  onDelete,
}: {
  profile: Agent;
  modelName: string | null;
  disabled: boolean;
  onStart: () => void;
  onDelete: () => void;
}) {
  return (
    <li className="prof">
      <div className="prof__main">
        <span className="prof__name">{profile.name}</span>
        <span className="prof__badges">
          <span className="badge">{profile.adapter}</span>
          <span className="badge badge--soft">{modelName ?? "Auto · coding"}</span>
        </span>
        <span className="prof__path numeric">{profile.workspace_path}</span>
        {profile.allowed_paths.length > 0 && (
          <span className="prof__extra numeric">
            + reads {profile.allowed_paths.join(", ")}
          </span>
        )}
      </div>
      <div className="prof__actions">
        <button type="button" className="prof__go" onClick={onStart} disabled={disabled}>
          New session
        </button>
        <button
          type="button"
          className="prof__del"
          onClick={onDelete}
          disabled={disabled}
          aria-label={`Delete ${profile.name}`}
        >
          Delete
        </button>
      </div>
    </li>
  );
}

function NewProfileForm({
  codingModels,
  onCreated,
}: {
  codingModels: { id: string; name: string }[];
  onCreated: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [allowed, setAllowed] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !workspace.trim() || busy) return;
    setBusy(true);
    setErr(null);
    try {
      await createAgent({
        name: name.trim(),
        adapter: "opencode",
        model_id: modelId === "auto" ? null : modelId,
        workspace_path: workspace.trim(),
        allowed_paths: allowed
          .split(/[,\n]/)
          .map((s) => s.trim())
          .filter(Boolean),
      });
      setName("");
      setWorkspace("");
      setAllowed("");
      setModelId("auto");
      setOpen(false);
      onCreated();
    } catch (e2) {
      setErr(e2 instanceof Error ? e2.message : String(e2));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <button type="button" className="agents__addbtn" onClick={() => setOpen(true)}>
        + New profile
      </button>
    );
  }

  return (
    <form className="profform" onSubmit={submit}>
      <label className="profform__field">
        <span>Name</span>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Repo coder"
        />
      </label>

      <label className="profform__field">
        <span>Runtime</span>
        <select value="opencode" disabled>
          <option value="opencode">OpenCode</option>
          <option value="hermes">Hermes (Phase 5.4)</option>
        </select>
      </label>

      <label className="profform__field">
        <span>Coding model</span>
        <select value={modelId} onChange={(e) => setModelId(e.target.value)}>
          <option value="auto">Auto — most-recently-used “coding” model</option>
          {codingModels.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
      </label>
      {codingModels.length === 0 && (
        <p className="muted">
          No model has the “coding” role yet — import a coding GGUF (e.g. Qwen2.5-Coder) on the
          Models tab and tick “coding”.
        </p>
      )}

      <label className="profform__field">
        <span>Workspace folder</span>
        <input
          type="text"
          value={workspace}
          onChange={(e) => setWorkspace(e.target.value)}
          placeholder="E:\\projects\\my-repo"
          spellCheck={false}
        />
      </label>

      <label className="profform__field">
        <span>Extra read-only folders (optional, comma-separated)</span>
        <input
          type="text"
          value={allowed}
          onChange={(e) => setAllowed(e.target.value)}
          placeholder="E:\\shared\\lib"
          spellCheck={false}
        />
      </label>

      <p className="muted">
        The agent may run shell commands and edit files — every command and every edit asks for
        your approval, edits stay inside the workspace, and it has no network access.
      </p>

      <div className="profform__buttons">
        <button type="submit" disabled={busy || !name.trim() || !workspace.trim()}>
          {busy ? "Creating…" : "Create profile"}
        </button>
        <button type="button" className="profform__cancel" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
      {err && <p className="agents__err">{err}</p>}
    </form>
  );
}

function SessionPanel({
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
        </header>
        <p className="muted">
          Pick a profile and hit <strong>New session</strong>. The transcript, tool calls and
          approval prompts show up here.
        </p>
      </section>
    );
  }

  const state = detail?.session.state ?? "starting";
  const stoppable = !TERMINAL.includes(state);
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
          </span>
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
        {detail?.session.state === "failed" && detail.session.error_text && (
          <div className="tblock tblock--error">{detail.session.error_text}</div>
        )}
      </div>

      {actionError && <p className="agents__err">{actionError}</p>}

      {!TERMINAL.includes(state) && (
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

      {TERMINAL.includes(state) && (
        <button type="button" className="agents__newbtn" onClick={onClosed}>
          Close transcript
        </button>
      )}
    </section>
  );
}

// --- transcript blocks ------------------------------------------------------

type TextBlock = { block: "text"; text: string };
type ToolBlock = {
  block: "tool";
  id: string;
  name: string;
  status: ToolStatus;
  command: string | null;
  output: string | null;
};
type PermissionBlock = {
  block: "permission";
  id: string;
  kind: string;
  summary: string;
  always: string | null;
};
type IdleBlock = { block: "idle" };
type ErrorBlock = { block: "error"; message: string };
type Block = TextBlock | ToolBlock | PermissionBlock | IdleBlock | ErrorBlock;

function commandOf(input: unknown): string | null {
  if (input && typeof input === "object" && "command" in input) {
    const c = (input as { command?: unknown }).command;
    if (typeof c === "string") return c;
  }
  if (input == null) return null;
  try {
    const s = JSON.stringify(input);
    return s === "{}" ? null : s;
  } catch {
    return null;
  }
}

/** Fold the raw event stream into a readable transcript: consecutive text
 *  deltas merge, a tool call updates in place by id, a permission ask shows
 *  once. */
function groupEvents(events: { payload: AgentEventPayload }[]): Block[] {
  const blocks: Block[] = [];
  for (const { payload } of events) {
    if (payload.type === "text") {
      const last = blocks[blocks.length - 1];
      if (last && last.block === "text") last.text += payload.text;
      else if (payload.text) blocks.push({ block: "text", text: payload.text });
    } else if (payload.type === "tool") {
      const existing = blocks.find(
        (b): b is ToolBlock => b.block === "tool" && b.id === payload.id,
      );
      const command = commandOf(payload.input);
      if (existing) {
        existing.status = payload.status;
        if (command) existing.command = command;
        if (payload.output != null) existing.output = payload.output;
      } else {
        blocks.push({
          block: "tool",
          id: payload.id || `t${blocks.length}`,
          name: payload.name,
          status: payload.status,
          command,
          output: payload.output ?? null,
        });
      }
    } else if (payload.type === "permission") {
      if (!blocks.some((b) => b.block === "permission" && b.id === payload.id)) {
        blocks.push({
          block: "permission",
          id: payload.id,
          kind: payload.kind,
          summary: payload.summary,
          always: payload.always_pattern ?? null,
        });
      }
    } else if (payload.type === "idle") {
      if (blocks[blocks.length - 1]?.block !== "idle") blocks.push({ block: "idle" });
    } else if (payload.type === "error") {
      blocks.push({ block: "error", message: payload.message });
    }
  }
  return blocks;
}

function TranscriptBlock({
  block,
  answered,
  onAnswer,
}: {
  block: Block;
  answered: boolean;
  onAnswer: (requestId: string, decision: PermissionDecision) => void;
}) {
  if (block.block === "text") {
    return <div className="tblock tblock--text">{block.text}</div>;
  }
  if (block.block === "idle") {
    return <div className="tblock tblock--idle">— idle —</div>;
  }
  if (block.block === "error") {
    return <div className="tblock tblock--error">{block.message}</div>;
  }
  if (block.block === "tool") {
    return (
      <div className="tool" data-status={block.status}>
        <div className="tool__head">
          <span className="tool__name">{block.name}</span>
          <span className="tool__status">{block.status}</span>
        </div>
        {block.command && <pre className="tool__cmd">{block.command}</pre>}
        {block.output && <pre className="tool__out">{block.output}</pre>}
      </div>
    );
  }
  // permission
  return (
    <div className="approval" data-answered={answered}>
      <div className="approval__head">
        <span className="badge badge--warn">approval</span>
        <span className="approval__kind">{block.kind}</span>
      </div>
      <pre className="approval__cmd">{block.summary}</pre>
      <div className="approval__actions">
        <button
          type="button"
          className="approval__allow"
          disabled={answered}
          onClick={() => onAnswer(block.id, "allow_once")}
        >
          Allow once
        </button>
        {block.always && (
          <button
            type="button"
            disabled={answered}
            onClick={() => onAnswer(block.id, "allow_always")}
            title={`Always allow: ${block.always}`}
          >
            Always
          </button>
        )}
        <button
          type="button"
          className="approval__deny"
          disabled={answered}
          onClick={() => onAnswer(block.id, "deny")}
        >
          Deny
        </button>
      </div>
    </div>
  );
}
