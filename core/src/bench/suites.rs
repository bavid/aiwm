//! Built-in benchmark suites — small, fixed sets of prompts the `bench` job can
//! run instead of the single default prompt.
//!
//! **The ids are versioned on purpose.** Changing a prompt's text, adding or
//! removing a prompt, or changing a suite's `max_tokens` changes the numbers it
//! produces, so it means a *new* id (`chat-v2`) rather than an edit to the old
//! one — stored rows keep pointing at the exact prompt set they were measured
//! with and stay comparable.
//!
//! `coding-v1` measures **throughput on coding-shaped output** (denser tokens,
//! more punctuation and identifiers than prose), **not** correctness: nothing
//! here runs or checks the generated code (ADR-024 — there is no local quality
//! benchmark). The prompts were written for this project — no text is copied
//! from HumanEval, MBPP, Exercism or any other benchmark, which keeps the
//! licence question simple. The task *shapes* are deliberately ordinary ones
//! (explain, summarise, rewrite, implement, debug); since only speed is
//! measured, a model having seen something similar in training does not change
//! what the numbers mean.

use serde::Serialize;

/// One prompt inside a [`Suite`].
#[derive(Debug, Clone, Serialize)]
pub struct SuitePrompt {
    /// Stable id; unique across *all* suites, and the key used in a stored
    /// benchmark's per-prompt detail.
    pub id: &'static str,
    /// Short label for the UI and the job's event trail.
    pub title: &'static str,
    /// The text sent to the model verbatim.
    pub text: &'static str,
}

/// A fixed, versioned set of prompts run at a fixed generation length.
#[derive(Debug, Clone, Serialize)]
pub struct Suite {
    /// Versioned id, e.g. `chat-v1`.
    pub id: &'static str,
    pub title: &'static str,
    /// What the suite measures, in the honest sense — shown in the UI.
    pub description: &'static str,
    /// Tokens generated per pass; inside `bench`'s own `16..=512` limits.
    pub max_tokens: i32,
    pub prompts: &'static [SuitePrompt],
}

const CHAT_V1_PROMPTS: &[SuitePrompt] = &[
    SuitePrompt {
        id: "chat-v1-explain",
        title: "Explain a concept",
        text: "Explain what a write-ahead log is, why databases keep one, and what happens \
               to it while a database recovers from a crash. Write three short paragraphs \
               for a reader who can program but has never worked on a database.",
    },
    SuitePrompt {
        id: "chat-v1-summarise",
        title: "Summarise a paragraph",
        text: "Summarise the paragraph below in two sentences. Keep every number exactly \
               as it is written.\n\n\
               The workshop moved its Tuesday maintenance window from 09:00 to 06:00 after \
               three weeks in which the morning delivery run kept overlapping with the lift \
               being out of service. The earlier window costs the team about 40 minutes of \
               overtime per week, but it removed 11 of the 14 late deliveries recorded in \
               the previous quarter, and the two apprentices who used to wait around for the \
               lift now start their shift on the shop floor instead.",
    },
    SuitePrompt {
        id: "chat-v1-rewrite",
        title: "Rewrite an email politely",
        text: "Rewrite the email below so that it stays firm about the deadline but reads \
               polite and professional. Keep it under 120 words and keep the subject line \
               informative.\n\n\
               Subject: still waiting\n\n\
               Hi Mara, this is the third time I am asking for the signed quote. We cannot \
               order the parts without it and the whole schedule slips because of you. I \
               need it today. Jonas",
    },
];

const CODING_V1_PROMPTS: &[SuitePrompt] = &[
    SuitePrompt {
        id: "coding-v1-write",
        title: "Write a function from a spec",
        text: "Write a Rust function with the signature \
               `fn merge_ranges(ranges: &[(u32, u32)]) -> Vec<(u32, u32)>`. The input holds \
               half-open ranges `[start, end)` in no particular order. Skip any range whose \
               `start` is not smaller than its `end`, merge ranges that overlap or touch, and \
               return the merged ranges sorted by `start`. Do not allocate a new vector inside \
               the loop. Add a doc comment and three `assert_eq!` lines that show the \
               behaviour, including the empty input.",
    },
    SuitePrompt {
        id: "coding-v1-fix",
        title: "Find and fix a bug",
        text: "The function below should return the index of the first element that is \
               strictly greater than `target`, or `None` when there is no such element. It \
               returns the wrong index for some inputs. Name the bug in one sentence, then \
               give the corrected function.\n\n\
               fn first_greater(sorted: &[i32], target: i32) -> Option<usize> {\n    \
               let mut lo = 0;\n    let mut hi = sorted.len();\n    \
               while lo < hi {\n        let mid = (lo + hi) / 2;\n        \
               if sorted[mid] > target {\n            lo = mid + 1;\n        \
               } else {\n            hi = mid;\n        }\n    }\n    \
               (lo < sorted.len()).then_some(lo)\n}",
    },
    SuitePrompt {
        id: "coding-v1-explain",
        title: "Explain code and propose tests",
        text: "Explain what the function below does, name the two inputs for which it \
               behaves surprisingly, and propose five test cases with their expected \
               results. Do not rewrite the function.\n\n\
               def bucket(values, size):\n    out = []\n    \
               for i, v in enumerate(values):\n        if i % size == 0:\n            \
               out.append([])\n        out[-1].append(v)\n    return out",
    },
];

const SUITES: &[Suite] = &[
    Suite {
        id: "chat-v1",
        title: "Chat (v1)",
        description: "Three everyday assistant prompts — explain, summarise, rewrite. \
                      Measures generation speed on prose, not answer quality.",
        max_tokens: 256,
        prompts: CHAT_V1_PROMPTS,
    },
    Suite {
        id: "coding-v1",
        title: "Coding (v1)",
        description: "Three programming prompts — write, debug, explain. Measures \
                      generation speed on code-shaped output; the code is never run or \
                      checked for correctness.",
        max_tokens: 384,
        prompts: CODING_V1_PROMPTS,
    },
];

/// Every built-in suite, in display order.
pub fn all() -> &'static [Suite] {
    SUITES
}

/// The suite with this exact id (case-sensitive), or `None`.
pub fn find(id: &str) -> Option<&'static Suite> {
    SUITES.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{MAX_MAX_TOKENS, MIN_MAX_TOKENS};
    use std::collections::HashSet;

    #[test]
    fn the_two_shipped_suites_are_present() {
        let ids: Vec<&str> = all().iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["chat-v1", "coding-v1"]);
    }

    #[test]
    fn every_id_is_unique_across_suites_and_prompts() {
        let mut seen: HashSet<&str> = HashSet::new();
        for suite in all() {
            assert!(seen.insert(suite.id), "duplicate suite id {}", suite.id);
            for prompt in suite.prompts {
                assert!(
                    seen.insert(prompt.id),
                    "duplicate prompt id {} in {}",
                    prompt.id,
                    suite.id
                );
            }
        }
    }

    #[test]
    fn suite_ids_are_versioned() {
        for suite in all() {
            let (stem, version) = suite
                .id
                .rsplit_once("-v")
                .unwrap_or_else(|| panic!("{} does not end in -v<N>", suite.id));
            assert!(!stem.is_empty(), "{}", suite.id);
            assert!(
                !version.is_empty() && version.chars().all(|c| c.is_ascii_digit()),
                "{} has a non-numeric version",
                suite.id
            );
        }
    }

    #[test]
    fn every_suite_has_three_non_empty_prompts() {
        for suite in all() {
            assert!(!suite.title.trim().is_empty(), "{}", suite.id);
            assert!(!suite.description.trim().is_empty(), "{}", suite.id);
            assert_eq!(suite.prompts.len(), 3, "{}", suite.id);
            for prompt in suite.prompts {
                assert!(!prompt.id.trim().is_empty(), "{}", suite.id);
                assert!(!prompt.title.trim().is_empty(), "{}", prompt.id);
                assert!(prompt.text.trim().len() > 40, "{} is too short", prompt.id);
            }
        }
    }

    #[test]
    fn max_tokens_stay_inside_the_bench_limits() {
        for suite in all() {
            assert!(
                (MIN_MAX_TOKENS..=MAX_MAX_TOKENS).contains(&suite.max_tokens),
                "{} asks for {} tokens",
                suite.id,
                suite.max_tokens
            );
        }
    }

    #[test]
    fn find_hits_known_ids_and_misses_everything_else() {
        assert_eq!(find("chat-v1").map(|s| s.id), Some("chat-v1"));
        assert_eq!(find("coding-v1").map(|s| s.id), Some("coding-v1"));
        assert!(find("chat-v2").is_none());
        assert!(find("").is_none());
        assert!(find("CHAT-V1").is_none(), "ids are case-sensitive");
    }

    #[test]
    fn suites_serialise_with_their_prompts() {
        let json = serde_json::to_value(find("chat-v1").unwrap()).unwrap();
        assert_eq!(json["id"], "chat-v1");
        assert_eq!(json["prompts"].as_array().unwrap().len(), 3);
        assert!(json["prompts"][0]["text"].as_str().unwrap().len() > 40);
    }
}
