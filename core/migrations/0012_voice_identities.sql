-- Named Dia voice-cloning identities (Voice tab) -- a short reference clip
-- plus its own transcript, set up once under a name (e.g. "Old Man Gareth")
-- and reused across many narration calls instead of re-picking a file and
-- re-typing the transcript every time. `reference_audio_path` always points
-- under AIWM's own data dir (`AppPaths::voice_identities_dir`) -- the file
-- the user picked is copied there once on creation (`core::voice_identity`),
-- never referenced in place, same principle as model import.
CREATE TABLE voice_identities (
    id                    TEXT PRIMARY KEY,   -- uuid v7
    name                  TEXT NOT NULL,
    reference_audio_path  TEXT NOT NULL,
    reference_transcript  TEXT NOT NULL,
    created_at            TEXT NOT NULL
) STRICT;
