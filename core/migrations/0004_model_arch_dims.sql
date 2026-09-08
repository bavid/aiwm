-- Architecture dimensions read from the GGUF header, feeding the VRAM /
-- KV-cache fit estimate (core::compat, slice 2.6). All nullable: an older
-- import, a non-GGUF format, or a stripped header may not carry them, and the
-- estimate falls back to a coarser guess.

ALTER TABLE models ADD COLUMN n_layers   INTEGER;   -- <arch>.block_count
ALTER TABLE models ADD COLUMN n_embd     INTEGER;   -- <arch>.embedding_length
ALTER TABLE models ADD COLUMN n_heads    INTEGER;   -- <arch>.attention.head_count
ALTER TABLE models ADD COLUMN n_kv_heads INTEGER;   -- <arch>.attention.head_count_kv
