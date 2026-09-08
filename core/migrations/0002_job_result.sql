-- Chat/completion jobs (Phase 2.4) stream their generated text into this column,
-- updated progressively as tokens arrive. NULL for jobs that produce a file
-- (those use `output_path`) or no output at all.
ALTER TABLE jobs ADD COLUMN result TEXT;
