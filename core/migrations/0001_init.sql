-- Schema v1 (see docs/PHASE_1_PLAN.md §4). Append-only history: state columns are
-- updated in place, everything else accretes rows. Later tables (agents,
-- benchmarks, downloads, collections) arrive as their own migrations.

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE runtimes (
    id          TEXT PRIMARY KEY,           -- 'llamacpp', 'comfyui', ...
    kind        TEXT NOT NULL,
    version     TEXT,
    install_path TEXT,
    state       TEXT NOT NULL,              -- not_installed | stopped | starting | running | error
    last_health TEXT,                       -- RFC3339
    last_error  TEXT
) STRICT;

CREATE TABLE models (
    id              TEXT PRIMARY KEY,       -- uuid v7
    publisher       TEXT,
    name            TEXT NOT NULL,
    family          TEXT,
    format          TEXT NOT NULL,          -- gguf | safetensors | ...
    quant           TEXT,
    arch            TEXT,
    param_count     INTEGER,
    file_path       TEXT NOT NULL,
    sha256          TEXT,
    size_bytes      INTEGER NOT NULL,
    ctx_max         INTEGER,
    vram_estimate_mb INTEGER,
    ram_estimate_mb  INTEGER,
    source          TEXT NOT NULL DEFAULT 'manual',
    source_revision TEXT,
    imported_at     TEXT NOT NULL,          -- RFC3339
    last_used_at    TEXT,
    use_count       INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE UNIQUE INDEX idx_models_file_path ON models(file_path);

CREATE TABLE model_roles (
    model_id TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    role     TEXT NOT NULL,                 -- coding | chat | upscaler | base_diffusion | ...
    PRIMARY KEY (model_id, role)
) STRICT;

CREATE TABLE model_links (
    model_id   TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    runtime_id TEXT NOT NULL REFERENCES runtimes(id),
    strategy   TEXT NOT NULL,               -- junction | copy | import
    link_path  TEXT NOT NULL,
    PRIMARY KEY (model_id, runtime_id)
) STRICT;

CREATE TABLE jobs (
    id          TEXT PRIMARY KEY,           -- uuid v7
    type        TEXT NOT NULL,              -- 'chat' | 'noop' (Phase 1)
    capability  TEXT,
    state       TEXT NOT NULL,              -- queued | scheduled | preparing | running | post | completed | failed | blocked | cancelled
    params_json TEXT NOT NULL,
    runtime_id  TEXT REFERENCES runtimes(id),
    model_id    TEXT REFERENCES models(id),
    created_at  TEXT NOT NULL,
    started_at  TEXT,
    finished_at TEXT,
    error_text  TEXT,
    output_path TEXT
) STRICT;

CREATE TABLE job_events (
    job_id  TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    ts      TEXT NOT NULL,
    level   TEXT NOT NULL,                  -- info | warn | error
    message TEXT NOT NULL
) STRICT;

CREATE INDEX idx_job_events_job ON job_events(job_id, ts);
CREATE INDEX idx_jobs_state ON jobs(state);
