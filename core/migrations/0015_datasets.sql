-- 0015_datasets.sql
-- A dataset is the durable object a training run (Plan 2) points at: a
-- curated set of frames or clips, reusable across many runs. Until now a
-- dataset_prep job's frames were only reachable through that job; the job
-- stays as the *producer* (prep_job_id, nulled if the job is deleted) while
-- the dataset itself survives.
CREATE TABLE datasets (
    id            TEXT PRIMARY KEY,                     -- uuid v7
    name          TEXT NOT NULL,
    mode          TEXT NOT NULL CHECK (mode IN ('frames', 'clips')),
    source_root   TEXT NOT NULL,
    trigger_word  TEXT NOT NULL DEFAULT '',             -- dataset-wide style trigger, in every caption
    prep_job_id   TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    export_dir    TEXT,                                 -- last export destination, NULL until exported
    created_at    TEXT NOT NULL
) STRICT;

-- Frames now belong to a dataset (cascade: no dataset, no frames). Rejected
-- frames are stored too, with the reason, so the curation grid can show and
-- restore them ('' = kept). Clip mode stores one row per video with its
-- preview still as frame_path and the clip itself as source_path.
ALTER TABLE dataset_frames ADD COLUMN dataset_id TEXT REFERENCES datasets(id) ON DELETE CASCADE;
ALTER TABLE dataset_frames ADD COLUMN rejection_reason TEXT NOT NULL DEFAULT '';
ALTER TABLE dataset_frames ADD COLUMN duration_secs REAL;
ALTER TABLE dataset_frames ADD COLUMN clip_start_secs REAL;
ALTER TABLE dataset_frames ADD COLUMN clip_end_secs REAL;
CREATE INDEX idx_dataset_frames_dataset ON dataset_frames(dataset_id);

-- A concept: a thing the user wants the LoRA to learn by name. `token` is
-- the trigger with no prior meaning in the base model ("kenji_xy");
-- `description` is optional extra caption text appended with it.
CREATE TABLE dataset_concepts (
    id          TEXT PRIMARY KEY,
    dataset_id  TEXT NOT NULL REFERENCES datasets(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    token       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    UNIQUE (dataset_id, token)
) STRICT;

-- Which frames show which concept. Never denormalised into the caption
-- column: export composes the final caption from these parts.
CREATE TABLE frame_concepts (
    frame_id   TEXT NOT NULL REFERENCES dataset_frames(id) ON DELETE CASCADE,
    concept_id TEXT NOT NULL REFERENCES dataset_concepts(id) ON DELETE CASCADE,
    PRIMARY KEY (frame_id, concept_id)
) STRICT;

CREATE INDEX idx_frame_concepts_concept ON frame_concepts(concept_id);
