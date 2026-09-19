-- 0020_training_run_lineage.sql
-- Plan 11: a run can continue an existing library LoRA instead of starting
-- from scratch. `init_lora_model_id` is that LoRA (NULL = from scratch); a
-- LoRA's lineage is the chain run -> init_lora_model_id -> that model's
-- `source = training:<id>` -> its run -> ..., computed on read. If the
-- source LoRA leaves the library the run keeps its own result and history.
ALTER TABLE training_runs ADD COLUMN init_lora_model_id TEXT REFERENCES models(id) ON DELETE SET NULL;
-- How many images/clips the trainer was actually fed at start (the export
-- folder's media count). NULL for rows older than this migration.
ALTER TABLE training_runs ADD COLUMN image_count INTEGER;
