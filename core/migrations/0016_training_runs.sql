-- 0016_training_runs.sql
-- A training run is NOT a job: it is a detached ai-toolkit process that must
-- survive app restarts (JobEngine::recover marks running jobs failed). State,
-- progress and the PID live here; the process writes checkpoints/samples/log
-- into work_dir, which is the source of truth for progress.
CREATE TABLE training_runs (
    id                 TEXT PRIMARY KEY,                       -- uuid v7
    name               TEXT NOT NULL,
    profile_family     TEXT NOT NULL,                          -- TrainingProfile.family
    target_model_id    TEXT REFERENCES models(id) ON DELETE SET NULL,
    dataset_id         TEXT REFERENCES datasets(id) ON DELETE SET NULL,
    data_kind          TEXT NOT NULL CHECK (data_kind IN ('frames', 'clips')),
    trigger_word       TEXT NOT NULL DEFAULT '',
    preset             TEXT NOT NULL CHECK (preset IN ('fast', 'balanced', 'thorough')),
    hyperparams_json   TEXT NOT NULL DEFAULT '{}',             -- rank/lr/resolution/steps overrides
    sample_prompts_json TEXT NOT NULL DEFAULT '[]',
    state              TEXT NOT NULL CHECK (state IN ('preparing','running','paused','interrupted','resuming','finishing','completed','failed','cancelled')),
    step               INTEGER NOT NULL DEFAULT 0,
    total_steps        INTEGER NOT NULL DEFAULT 0,
    last_loss          REAL,
    last_checkpoint_at TEXT,
    pid                INTEGER,
    work_dir           TEXT NOT NULL,
    result_model_id    TEXT REFERENCES models(id) ON DELETE SET NULL,
    error_text         TEXT,
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    started_at         TEXT,
    finished_at        TEXT
) STRICT;
CREATE INDEX idx_training_runs_state ON training_runs(state);
CREATE INDEX idx_training_runs_dataset ON training_runs(dataset_id);
