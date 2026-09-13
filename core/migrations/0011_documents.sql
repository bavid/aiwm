-- Documents attached to a chat session for local RAG (7.x): a session can
-- have zero or more documents; each is split into chunks at import time and
-- scored against the live prompt to ground the model's answer (lexical
-- keyword search -- no embedding model, see ADR discussion in docs/TODO.md).
--
-- Unlike `sessions.delete` (which orphans jobs rather than deleting them),
-- documents cascade-delete with their session: they are input material with
-- no standalone value the way a generated image or chat transcript has, so
-- there is nothing to preserve once the session that attached them is gone.
CREATE TABLE documents (
    id          TEXT PRIMARY KEY,           -- uuid v7
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,              -- original file name, for display
    source_path TEXT NOT NULL,              -- where it was imported from
    format      TEXT NOT NULL,              -- 'txt' | 'md'
    created_at  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_documents_session ON documents(session_id);

CREATE TABLE document_chunks (
    id           TEXT PRIMARY KEY,          -- uuid v7
    document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,          -- position within the document
    text         TEXT NOT NULL
) STRICT;

CREATE INDEX idx_document_chunks_document ON document_chunks(document_id);
