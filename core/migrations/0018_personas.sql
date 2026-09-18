-- Personas (spec `docs/superpowers/specs/2026-09-18-personas-design.md`): named
-- presets of name + one emoji icon + system prompt. One persona can be active
-- globally (settings key `chat.active_persona_id`); a chat session may override
-- it with another persona or with an explicit "no persona".
--
-- Purely additive, so an existing database keeps behaving exactly as before:
-- every session starts at `persona_mode = 'inherit'`, and with no global
-- persona set that resolves to "no persona" — today's request, byte for byte.
--
-- `system_prompt` is stored verbatim; the tool applies no content filter. The
-- only limits are technical (`core::persona::validate`).
CREATE TABLE personas (
    id             TEXT PRIMARY KEY,   -- uuid v7
    name           TEXT NOT NULL,
    icon           TEXT NOT NULL,      -- one emoji, stored as text
    system_prompt  TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
) STRICT;

-- The manage dialog lists personas by name.
CREATE INDEX idx_personas_name ON personas(name);

-- `sessions` is a STRICT table. SQLite accepts `ADD COLUMN` on one as long as
-- the column has a declared type and, when NOT NULL, a constant default -- both
-- hold here, so no table rebuild is needed.
--
-- `persona_mode` — 'inherit' (use the global persona) | 'none' (explicitly no
--                  persona for this chat) | 'persona' (use `persona_id`).
-- `persona_id`   — set only with mode 'persona'. Deliberately *not* a foreign
--                  key: a deleted persona is healed on read/delete
--                  (`core::persona::resolve`) rather than cascading, so a chat
--                  can never fail on a stale id and the mode never ends up
--                  pointing at a NULL.
ALTER TABLE sessions ADD COLUMN persona_mode TEXT NOT NULL DEFAULT 'inherit';
ALTER TABLE sessions ADD COLUMN persona_id TEXT;
