-- Benchmark tab: a `bench` job may now run a versioned prompt suite
-- (`core/src/bench/suites.rs`) instead of the single default prompt.
--
-- `suite`       — the suite id the row was measured with (e.g. 'chat-v1'); NULL
--                 for the older "quick test" rows from the Model Library, which
--                 stay valid and comparable among themselves.
-- `detail_json` — JSON array of per-prompt results
--                 [{ "prompt_id", "tokens", "gen_tps", "prompt_tps" }, ...];
--                 NULL when there is no suite. Stored as text because SQLite has
--                 no JSON column type and nothing here is queried by key.
--
-- Suite ids are versioned on purpose: changing a prompt means a new id, so rows
-- carrying the same `suite` were always produced by the same prompt set.

ALTER TABLE benchmarks ADD COLUMN suite TEXT;
ALTER TABLE benchmarks ADD COLUMN detail_json TEXT;

-- The Benchmark tab's comparison table reads "newest rows for this suite".
CREATE INDEX idx_benchmarks_suite ON benchmarks(suite, created_at);
