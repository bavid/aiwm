-- Phase 6 follow-up: a download can now carry the roles to stamp on the
-- model once it is imported (e.g. "chat,coding" for an agent pick from the
-- Models tab's Featured catalog). Comma-joined, empty string = none (the
-- prior behaviour for every download queued before this migration).

ALTER TABLE downloads ADD COLUMN roles TEXT NOT NULL DEFAULT '';
