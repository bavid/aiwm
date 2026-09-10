-- Phase 6 (Model-Manager v2, slice 6.5): local micro-benchmarks. One row per
-- "Test model" run. Everything here is measured on THIS machine — tokens/sec,
-- load time, VRAM/RAM peak, consistency across N passes. `overall_score` is an
-- openly-declared weighted heuristic (speed + fit + stability), NOT a quality
-- score — there is no cheap local quality benchmark (ADR-024).

CREATE TABLE benchmarks (
    id               TEXT PRIMARY KEY,            -- uuid v7
    model_id         TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    job_id           TEXT,                        -- the `bench` job that produced this
    kind             TEXT NOT NULL DEFAULT 'llm', -- 'llm' now; 'image' / 'video' later
    runs             INTEGER NOT NULL,            -- generation passes averaged
    prompt_tps       REAL,                        -- prompt (prefill) tokens/sec, mean
    gen_tps          REAL,                        -- generation tokens/sec, mean
    load_ms          INTEGER,                     -- cold load time; NULL when already resident
    vram_peak_mb     INTEGER,                     -- NVML peak during the run; NULL without a GPU
    ram_peak_mb      INTEGER,                     -- sysinfo peak during the run
    stability_score  REAL NOT NULL,               -- 0..1, 1 = perfectly consistent tok/s
    overall_score    INTEGER NOT NULL,            -- 0..100 declared heuristic
    notes            TEXT,
    created_at       TEXT NOT NULL
) STRICT;

CREATE INDEX idx_benchmarks_model ON benchmarks(model_id, created_at);
