-- Phase 6 (Model-Manager v2, slice 6.9): free-form tags / collections on
-- library models. A model can carry any number of short labels ("favourite",
-- "coding", "keep") the user assigns; the UI filters the Model Library by them.

CREATE TABLE model_tags (
    model_id TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    tag      TEXT NOT NULL,
    PRIMARY KEY (model_id, tag)
) STRICT;

CREATE INDEX idx_model_tags_tag ON model_tags(tag);
