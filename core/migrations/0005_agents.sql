-- Phase 5 (Agents, slice 5.1a): profiles for the managed agent runtimes
-- (OpenCode first — ADR-021; Hermes later) and the long-running sessions they
-- drive. The agent's own state lives on the filesystem (OpenCode's
-- ~/.local/share/opencode, Hermes' profile dir); this tracks what the app
-- orchestrates plus the event transcript the UI polls.

CREATE TABLE agents (
    id                 TEXT PRIMARY KEY,          -- uuid v7
    name               TEXT NOT NULL,
    adapter            TEXT NOT NULL,             -- 'opencode' | 'hermes'
    model_id           TEXT,                      -- soft ref to models(id); NULL = Auto over the 'coding' role
    workspace_path     TEXT NOT NULL,             -- the directory the agent runs in (path-allowlist root)
    allowed_paths_json TEXT NOT NULL DEFAULT '[]',-- extra roots the agent may read
    toolset_json       TEXT,                      -- NULL = the adapter's default toolset
    created_at         TEXT NOT NULL
) STRICT;

CREATE TABLE agent_sessions (
    id                 TEXT PRIMARY KEY,          -- uuid v7
    agent_id           TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    adapter_session_id TEXT,                      -- the id the agent runtime assigned
    state              TEXT NOT NULL,             -- starting | idle | working | awaiting_approval | stopped | failed
    error_text         TEXT,
    checkpoint_path    TEXT,                      -- for resume after a crash / restart (slice 5.5)
    started_at         TEXT NOT NULL,
    ended_at           TEXT
) STRICT;

CREATE INDEX idx_agent_sessions_agent ON agent_sessions(agent_id, started_at);

-- Append-only transcript: text deltas, tool calls, permission requests, errors,
-- state changes. `payload_json` shape depends on `kind` (see core::agent).
CREATE TABLE agent_session_events (
    session_id   TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
    ts           TEXT NOT NULL,
    kind         TEXT NOT NULL,                   -- text | tool | permission | error | state | idle
    payload_json TEXT NOT NULL
) STRICT;

CREATE INDEX idx_agent_session_events_session ON agent_session_events(session_id, ts);
