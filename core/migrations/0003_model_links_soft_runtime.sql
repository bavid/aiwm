-- `model_links.runtime_id` was a FK to `runtimes(id)`, but a model can be linked
-- to a runtime before that runtime is installed / has a `runtimes` row. Soft-ref
-- it (like `jobs.runtime_id`) so the model importer can record links freely.
-- The `models` FK (cascade on model delete) stays. Table is still empty here.

CREATE TABLE model_links_v2 (
    model_id   TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    runtime_id TEXT NOT NULL,                 -- soft ref: 'llamacpp', 'comfyui', ...
    strategy   TEXT NOT NULL,                 -- passthrough | junction | hardlink | copy
    link_path  TEXT NOT NULL,                 -- the path the runtime hands its loader
    PRIMARY KEY (model_id, runtime_id)
) STRICT;

INSERT INTO model_links_v2 SELECT model_id, runtime_id, strategy, link_path FROM model_links;
DROP TABLE model_links;
ALTER TABLE model_links_v2 RENAME TO model_links;
