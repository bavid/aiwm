-- Plan 14: a download remembers what it is, so the import can record it on
-- the model. origin = 'civitai:<modelId>/<versionId>' | 'hf:<repo>@<rev>'
-- (becomes models.source when the model had none); base_family = the
-- registry id the source's base label maps to (NULL when unknown — never
-- guessed); family_source = 'civitai' | 'hf'. All NULL for downloads queued
-- without source metadata (catalog stacks, older clients).
ALTER TABLE downloads ADD COLUMN origin TEXT;
ALTER TABLE downloads ADD COLUMN base_family TEXT;
ALTER TABLE downloads ADD COLUMN family_source TEXT;
