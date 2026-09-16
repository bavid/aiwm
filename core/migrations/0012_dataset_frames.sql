-- Frames produced by a `dataset_prep` job (the dataset-prep half of the
-- "Lokale KI-Trainings-Engine" — see docs/TODO.md; the training-orchestrator
-- half is explicitly out of scope). One row per kept frame (post blur/near-
-- duplicate filtering): a still image on disk, its folder-derived tag, and
-- its auto/edited caption, ready for the curation UI to review before
-- `dataset::export_dataset` writes the final `NNNN.png`/`NNNN.txt` pairs.
--
-- Cascade-deletes with its job, same reasoning `documents` cascading with its
-- session used in 0011: a frame has no standalone value once the job that
-- produced it is gone (unlike a generated image/video job's own output,
-- which the gallery keeps).
CREATE TABLE dataset_frames (
    id             TEXT PRIMARY KEY,           -- uuid v7
    job_id         TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    tag            TEXT NOT NULL,              -- folder name the source lived under
    source_path    TEXT NOT NULL,              -- original video/image this frame came from
    frame_path     TEXT NOT NULL,              -- extracted/copied still on disk
    timestamp_secs REAL,                       -- position within the source video; NULL for a still image
    caption        TEXT NOT NULL DEFAULT '',
    caption_engine TEXT NOT NULL DEFAULT '',   -- 'florence2' | 'qwen2.5-vl' | '' (not captioned yet)
    excluded       INTEGER NOT NULL DEFAULT 0, -- curator dropped this frame from the export
    created_at     TEXT NOT NULL
) STRICT;

CREATE INDEX idx_dataset_frames_job ON dataset_frames(job_id);
