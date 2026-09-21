-- Plan 14: the base family a model is made for, as a registry id
-- (core::model::family::FAMILIES — sdxl, pony, flux2-klein-9b, …).
-- Deliberately a column of its own: models.family keeps its legacy/runtime
-- meaning (flux, flux2, wan, ltx, …) that the image/video recipes, the
-- trainer preflight and the LoRA pickers compare against, and is never
-- rewritten by this feature. family_source (0022) records how base_family
-- was decided; a 'user' value is only ever replaced by another 'user' value
-- (ModelRepo::set_base_family).
ALTER TABLE models ADD COLUMN base_family TEXT;
