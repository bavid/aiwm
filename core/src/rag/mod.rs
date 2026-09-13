//! Local document retrieval for chat (7.x) — lexical keyword search (BM25),
//! not semantic embeddings. `LlamaCppAdapter` runs exactly one resident model
//! at a time (`core::runtime::llamacpp`), so computing query/chunk embeddings
//! would mean either swapping the chat model out on every turn or running a
//! second concurrent server — both a real cost for a "ground my answer in
//! these files" feature that BM25 sidesteps entirely: no embedding model, no
//! runtime changes, and a well-understood, real retrieval baseline.
//!
//! Chunking is fixed-size word windows with overlap (words, not a real
//! tokenizer — dependency-free and close enough for grounding chat answers).
//! Scoring is textbook BM25 (Robertson/Sparck Jones), computed fresh per chat
//! turn over whatever chunks are attached to the session — see the scale
//! reasoning in the scoping notes: brute-force is fine into the thousands of
//! chunks, far past what one chat session's attachments would ever hold.

use std::collections::HashMap;
use std::path::Path;

use crate::{CoreError, Result};

/// File extensions this can ingest. Binary formats (PDF, DOCX, …) are
/// explicitly out of scope for v1 — see the scoping notes.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["txt", "md"];

/// Words per chunk. ~300 words is roughly 400-450 tokens for English prose —
/// a paragraph or two, small enough that several chunks plus the question
/// still fit comfortably in a chat model's context.
pub const DEFAULT_CHUNK_WORDS: usize = 300;
/// ~13% overlap — enough that an idea split across a chunk boundary still
/// appears whole in at least one chunk, without duplicating most of the text.
pub const DEFAULT_OVERLAP_WORDS: usize = 40;
/// Retrieved chunks per chat turn. Enough context to ground an answer without
/// crowding out the conversation itself.
pub const DEFAULT_TOP_K: usize = 4;

fn rag_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "rag".into(),
        message: msg.to_string(),
    }
}

/// Read a `.txt`/`.md` file as text. Any other extension is rejected with a
/// plain-language error rather than silently mis-parsing binary content.
pub fn read_document(path: &Path) -> Result<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !ext
        .as_deref()
        .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e))
    {
        return Err(rag_err(format!(
            "only .txt and .md documents are supported right now — got {}",
            path.display()
        )));
    }
    std::fs::read_to_string(path).map_err(|e| rag_err(format!("read {}: {e}", path.display())))
}

/// Split `text` into fixed-size, overlapping word windows. Empty/whitespace-only
/// text yields no chunks.
pub fn chunk_text(text: &str, chunk_words: usize, overlap_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || chunk_words == 0 {
        return Vec::new();
    }
    let step = chunk_words.saturating_sub(overlap_words).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    loop {
        let end = (start + chunk_words).min(words.len());
        chunks.push(words[start..end].join(" "));
        if end == words.len() {
            break;
        }
        start += step;
    }
    chunks
}

/// Lowercase, split on anything that isn't alphanumeric.
fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

const BM25_K1: f64 = 1.5;
const BM25_B: f64 = 0.75;

/// Rank `chunks` against `query` by BM25 score, returning the indices of the
/// top `k` (highest score first). A chunk that shares no term with the query
/// scores `0` and is excluded — an unrelated chunk is worse than no chunk at
/// all in the prompt.
pub fn top_k_by_relevance(chunks: &[String], query: &str, k: usize) -> Vec<usize> {
    if chunks.is_empty() || k == 0 {
        return Vec::new();
    }
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return Vec::new();
    }

    let docs: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(c)).collect();
    let n = docs.len() as f64;
    let avg_len = (docs.iter().map(Vec::len).sum::<usize>() as f64 / n).max(1.0);

    let mut unique_terms = query_terms.clone();
    unique_terms.sort();
    unique_terms.dedup();
    let idf: HashMap<&str, f64> = unique_terms
        .iter()
        .map(|term| {
            let n_containing = docs.iter().filter(|d| d.iter().any(|w| w == term)).count() as f64;
            let idf = ((n - n_containing + 0.5) / (n_containing + 0.5) + 1.0).ln();
            (term.as_str(), idf)
        })
        .collect();

    let mut scores: Vec<(usize, f64)> = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| {
            let len = doc.len() as f64;
            let score: f64 = query_terms
                .iter()
                .map(|term| {
                    let tf = doc.iter().filter(|w| *w == term).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let term_idf = idf.get(term.as_str()).copied().unwrap_or(0.0);
                    term_idf * (tf * (BM25_K1 + 1.0))
                        / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * len / avg_len))
                })
                .sum();
            (i, score)
        })
        .filter(|&(_, score)| score > 0.0)
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(k);
    scores.into_iter().map(|(i, _)| i).collect()
}

/// Wrap retrieved chunks in a preamble the model reads before the question —
/// the injection point `capability::chat` uses. Returns `prompt` unchanged
/// when nothing was retrieved (no attached documents, or no chunk matched).
pub fn build_grounded_prompt(prompt: &str, retrieved: &[&str]) -> String {
    if retrieved.is_empty() {
        return prompt.to_string();
    }
    let context = retrieved
        .iter()
        .enumerate()
        .map(|(i, chunk)| format!("[{}] {chunk}", i + 1))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Use the following excerpts from attached documents to answer the question. \
         If they don't contain the answer, say so rather than guessing.\n\n\
         {context}\n\n\
         Question: {prompt}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_splits_into_overlapping_windows() {
        let words: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text, 4, 1);
        assert_eq!(chunks, vec!["0 1 2 3", "3 4 5 6", "6 7 8 9"]);
    }

    #[test]
    fn chunk_text_returns_one_chunk_when_shorter_than_the_window() {
        assert_eq!(chunk_text("a short line", 300, 40), vec!["a short line"]);
    }

    #[test]
    fn chunk_text_is_empty_for_blank_input() {
        assert!(chunk_text("   ", 300, 40).is_empty());
        assert!(chunk_text("", 300, 40).is_empty());
    }

    #[test]
    fn top_k_by_relevance_ranks_the_matching_chunk_first() {
        let chunks = vec![
            "the quick brown fox jumps over the lazy dog".to_string(),
            "SQLite is a small, fast, self-contained SQL database engine".to_string(),
            "the fox and the dog became unlikely friends in the story".to_string(),
        ];
        let top = top_k_by_relevance(&chunks, "fox and dog friendship", 2);
        assert_eq!(top, vec![2, 0]);
    }

    #[test]
    fn top_k_by_relevance_excludes_chunks_sharing_no_term() {
        let chunks = vec![
            "database engines and query planners".to_string(),
            "a recipe for sourdough bread".to_string(),
        ];
        let top = top_k_by_relevance(&chunks, "sourdough bread recipe", 5);
        assert_eq!(top, vec![1]);
    }

    #[test]
    fn top_k_by_relevance_is_empty_for_no_chunks_or_blank_query() {
        assert!(top_k_by_relevance(&[], "anything", 5).is_empty());
        assert!(top_k_by_relevance(&["some text".to_string()], "   ", 5).is_empty());
    }

    #[test]
    fn top_k_by_relevance_respects_k() {
        let chunks: Vec<String> = (0..10).map(|i| format!("apple chunk number {i}")).collect();
        assert_eq!(top_k_by_relevance(&chunks, "apple", 3).len(), 3);
    }

    #[test]
    fn build_grounded_prompt_wraps_retrieved_chunks() {
        let out = build_grounded_prompt("What is the deadline?", &["Ship by March 1."]);
        assert!(out.contains("Ship by March 1."));
        assert!(out.contains("What is the deadline?"));
        assert!(out.contains("[1]"));
    }

    #[test]
    fn build_grounded_prompt_passes_through_with_nothing_retrieved() {
        assert_eq!(build_grounded_prompt("hello", &[]), "hello");
    }

    #[test]
    fn read_document_rejects_unsupported_extensions() {
        let err = read_document(Path::new("report.pdf")).unwrap_err();
        assert!(err.to_string().contains(".txt and .md"));
    }

    #[test]
    fn read_document_reads_txt_and_md() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notes.md");
        std::fs::write(&path, "# Title\n\nBody text.").unwrap();
        assert_eq!(read_document(&path).unwrap(), "# Title\n\nBody text.");
    }
}
