-- Phase 6 (Model-Manager v2, slice 6.4): the download manager. One row per
-- download. The file streams to a staging path with HTTP-Range resume, is
-- verified against the expected SHA-256, then handed to import_model (which
-- moves it into the canonical store). Progress is a polled field
-- (bytes_done / state), not an event stream.

CREATE TABLE downloads (
    id           TEXT PRIMARY KEY,           -- uuid v7
    url          TEXT NOT NULL,
    filename     TEXT NOT NULL,              -- basename, for the staging path + the UI
    dest_path    TEXT NOT NULL,              -- <local_root>/.downloads/<id>/<filename>
    model_type   TEXT,                       -- passed to import_model (chat|checkpoint|…)
    sha256       TEXT,                       -- expected (lowercase hex); NULL = skip the check
    size_bytes   INTEGER,                    -- expected total; NULL until the response headers
    bytes_done   INTEGER NOT NULL DEFAULT 0,
    retries      INTEGER NOT NULL DEFAULT 0, -- transport retries so far (capped, then failed)
    state        TEXT NOT NULL,              -- queued | running | paused | verifying | done | failed
    error_text   TEXT,
    model_id     TEXT,                       -- models(id) after a successful import
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;

CREATE INDEX idx_downloads_state ON downloads(state, created_at);
