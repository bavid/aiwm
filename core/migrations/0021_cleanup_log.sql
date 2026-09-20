-- 0021_cleanup_log.sql
-- Plan 13: what the Settings "Cleanup" page deleted — one row per applied
-- entry (a media file, a dataset's discarded frames, an unclaimed work
-- folder, a run's folder, a cache, the old logs, a backup). Deliberately no
-- foreign key to jobs, datasets or runs: the log survives the rows it is
-- about. Only a real apply writes here; a dry run never does.
CREATE TABLE cleanup_log (
    id            TEXT PRIMARY KEY,                                            -- uuid v7
    ts            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), -- RFC 3339, UTC
    group_key     TEXT NOT NULL,                                               -- one of the scan's group keys
    entry_id      TEXT NOT NULL,                                               -- the scan's entry id
    entry_label   TEXT NOT NULL,                                               -- what the page showed
    deleted_files INTEGER NOT NULL,
    freed_bytes   INTEGER NOT NULL,
    removed_rows  INTEGER NOT NULL,                                            -- frame rows, else 0
    skipped_count INTEGER NOT NULL,
    detail_json   TEXT NOT NULL DEFAULT '{}'                                   -- skip reasons, paths (capped)
) STRICT;

CREATE INDEX idx_cleanup_log_ts ON cleanup_log(ts);
