-- Plan 14: how models.family was decided — civitai | hf | catalog | header |
-- name | user (core::model::family::FamilySource). NULL for rows written
-- before this migration, whose family came from the file name, the catalog or
-- the trainer. A 'user' value is only ever replaced by another 'user' value
-- (ModelRepo::set_family).
ALTER TABLE models ADD COLUMN family_source TEXT;
