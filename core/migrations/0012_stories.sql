-- Story Studio (Phase 1 -- text-and-plain-image MVP, see docs/TODO.md
-- "Story Studio"): a character-consistent illustrated story/comic builder.
-- Phase 1 deliberately ships WITHOUT the character-consistency machinery
-- (IP-Adapter, Phase 2) -- portrait/reference images are just whichever
-- `job_type=image` job the user picks, stored as a plain job_id. Story
-- Studio never creates a job itself; it only remembers which one a caller
-- chose, via a soft `REFERENCES jobs(id)`.
--
-- Structural rows (characters, npcs, locations, scenes, participants,
-- dialogue, scene_images, character_logs) cascade-delete with their Story --
-- they are the Story's own internal shape, not standalone generated
-- content, so nothing valuable is lost by keeping deletes total (same
-- reasoning as `documents` cascading with its session, 0011_documents.sql).
-- A generated image job itself is never deleted by any of this; only the
-- Story-side row pointing at it goes away (or, for a portrait/reference
-- image, just its job_id is nulled out).
CREATE TABLE stories (
    id         TEXT PRIMARY KEY,          -- uuid v7
    name       TEXT NOT NULL,
    setting    TEXT NOT NULL DEFAULT '',  -- era/setting, freeform
    art_style  TEXT NOT NULL DEFAULT '',  -- freeform; feeds every generation prompt
    premise    TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE locations (
    id               TEXT PRIMARY KEY,
    story_id         TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    reference_job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    created_at       TEXT NOT NULL
) STRICT;

CREATE INDEX idx_locations_story ON locations(story_id);

CREATE TABLE characters (
    id              TEXT PRIMARY KEY,
    story_id        TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    traits          TEXT NOT NULL DEFAULT '',
    backstory       TEXT NOT NULL DEFAULT '',
    alignment       TEXT NOT NULL DEFAULT '', -- role/alignment hint, e.g. "elf, friendly, skilled"
    portrait_job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    inventory       TEXT NOT NULL DEFAULT '[]', -- JSON array of strings
    created_at      TEXT NOT NULL
) STRICT;

CREATE INDEX idx_characters_story ON characters(story_id);

-- One character's relationship to another, from that character's own point
-- of view (A-trusts-B and B-resents-A are two independent rows, not one
-- symmetric edge). Kept normalized on purpose -- never denormalized into a
-- JSON blob on `characters` -- so the later "who talked with whom" workflow
-- graph (a deliberately-deferred later phase) can be reconstructed from real
-- rows instead of a restructure.
CREATE TABLE character_relationships (
    id                    TEXT PRIMARY KEY,
    character_id          TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    related_character_id  TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    note                  TEXT NOT NULL,
    created_at            TEXT NOT NULL
) STRICT;

CREATE INDEX idx_character_relationships_character ON character_relationships(character_id);

-- Deliberately lightweight -- NOT the full Character shape (Phase 1 scope
-- cut, see docs/TODO.md).
CREATE TABLE npcs (
    id          TEXT PRIMARY KEY,
    story_id    TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    role        TEXT NOT NULL DEFAULT '',
    location_id TEXT REFERENCES locations(id) ON DELETE SET NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_npcs_story ON npcs(story_id);

CREATE TABLE scenes (
    id          TEXT PRIMARY KEY,
    story_id    TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    location_id TEXT REFERENCES locations(id) ON DELETE SET NULL,
    narrative   TEXT NOT NULL DEFAULT '',
    redline     TEXT NOT NULL DEFAULT '', -- the short scenario prompt that led to this scene
    position    INTEGER NOT NULL,         -- ordering within the story's timeline
    created_at  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_scenes_story ON scenes(story_id, position);

CREATE TABLE scene_participants (
    scene_id     TEXT NOT NULL REFERENCES scenes(id) ON DELETE CASCADE,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    PRIMARY KEY (scene_id, character_id)
) STRICT;

CREATE INDEX idx_scene_participants_character ON scene_participants(character_id);

-- Dialogue is always rendered as UI overlay text, never baked into a
-- generated image -- explicit user requirement, AI text-in-images is
-- unreliable. `position` orders lines within the scene.
CREATE TABLE scene_dialogue_lines (
    id           TEXT PRIMARY KEY,
    scene_id     TEXT NOT NULL REFERENCES scenes(id) ON DELETE CASCADE,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    text         TEXT NOT NULL
) STRICT;

CREATE INDEX idx_scene_dialogue_scene ON scene_dialogue_lines(scene_id);

-- A Scene can have multiple generated images (regenerate / pick alternates);
-- exactly one is canonical at a time -- enforced by the partial unique index
-- below rather than application discipline alone.
CREATE TABLE scene_images (
    id           TEXT PRIMARY KEY,
    scene_id     TEXT NOT NULL REFERENCES scenes(id) ON DELETE CASCADE,
    job_id       TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    is_canonical INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL
) STRICT;

CREATE INDEX idx_scene_images_scene ON scene_images(scene_id);
CREATE UNIQUE INDEX idx_scene_images_one_canonical ON scene_images(scene_id) WHERE is_canonical = 1;

-- Append-only event stream per Character -- auto-appended whenever the
-- character participates in a new Scene (`SceneRepo::create` /
-- `SceneRepo::update`, for newly-added participants only). Cascades with the
-- Scene it describes: if the scene itself is removed, the event it
-- describes is gone too.
CREATE TABLE character_logs (
    id           TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    scene_id     TEXT NOT NULL REFERENCES scenes(id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL, -- doubles as the event timestamp
    text         TEXT NOT NULL
) STRICT;

CREATE INDEX idx_character_logs_character ON character_logs(character_id, created_at);
