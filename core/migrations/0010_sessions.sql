-- Sessions: named, switchable groupings of jobs for Chat/Image/Video (a
-- lightweight "project" concept, ChatGPT/Claude-style -- e.g. one session for
-- anime-style prompts, another for realism). A session carries no settings of
-- its own beyond a name: "remembers what you last used" is derived from its
-- most recent job's params at read time, not stored separately here (YAGNI --
-- no separate defaults blob until a real need for one shows up).
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,           -- uuid v7
    capability  TEXT NOT NULL,              -- 'chat' | 'image' | 'video'
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    archived_at TEXT                        -- NULL = active; hidden from the switcher once set
) STRICT;

CREATE INDEX idx_sessions_capability ON sessions(capability, created_at);

-- Deleting a session ungroups its jobs rather than deleting them -- nothing in
-- the Model Library / gallery / chat history is ever lost. NULL (the default
-- for every job before this column existed, and for any job never assigned to
-- a session) means "ungrouped", not an error.
ALTER TABLE jobs ADD COLUMN session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL;
CREATE INDEX idx_jobs_session ON jobs(session_id);
