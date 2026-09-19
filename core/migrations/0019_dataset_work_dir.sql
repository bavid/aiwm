-- 0019_dataset_work_dir.sql
-- Plan 10: a dataset's work folder can live anywhere the user chose, so its
-- absolute path is stored on the row (`<data_dir>/<prep_job_id>`). Every new
-- dataset records it, the default location included. Older rows stay NULL
-- and keep the derived `<datasets root>/<prep_job_id>`.
ALTER TABLE datasets ADD COLUMN work_dir TEXT;
