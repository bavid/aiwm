//! Unit tests for [`crate::bench`] — the pure scoring/aggregation helpers.
//! Tests that drive [`super::run`] against a stand-in llama-server live in
//! [`run`], split out to keep this file under the repo's line limit.

use super::*;

mod run;

#[test]
fn stability_of_identical_samples_is_one() {
    assert_eq!(stability_score(&[60.0, 60.0, 60.0]), 1.0);
    assert_eq!(stability_score(&[42.0]), 1.0);
    assert_eq!(stability_score(&[]), 1.0);
}

#[test]
fn stability_drops_as_samples_spread() {
    let tight = stability_score(&[60.0, 61.0, 59.0]);
    let loose = stability_score(&[60.0, 30.0, 90.0]);
    assert!(tight > 0.9, "{tight}");
    assert!(loose < 0.6, "{loose}");
    assert!(tight > loose);
}

#[test]
fn overall_score_rewards_speed_fit_and_stability() {
    let fast_fit = overall_score(80.0, 1.0, &FitVerdict::Green);
    let slow_fit = overall_score(10.0, 1.0, &FitVerdict::Green);
    let fast_tight = overall_score(
        80.0,
        1.0,
        &FitVerdict::Yellow {
            reason: "tight".into(),
        },
    );
    let fast_nofit = overall_score(
        80.0,
        1.0,
        &FitVerdict::Red {
            reason: "over budget".into(),
        },
    );

    assert!(fast_fit > slow_fit);
    assert!(fast_fit > fast_tight);
    assert!(fast_tight > fast_nofit);
    assert_eq!(fast_fit, 100);
    // A model that won't fit is capped low even when it's fast.
    assert!(fast_nofit < 45, "{fast_nofit}");
}

#[test]
fn overall_score_clamps_a_wild_tps() {
    assert_eq!(overall_score(100_000.0, 1.0, &FitVerdict::Green), 100);
    assert_eq!(overall_score(-5.0, 0.0, &FitVerdict::Unknown), 0);
}

#[test]
fn request_from_params_defaults_and_clamps() {
    let d = BenchRequest::from_params(&serde_json::json!({}));
    assert_eq!(d.runs, DEFAULT_RUNS);
    assert_eq!(d.max_tokens, DEFAULT_MAX_TOKENS);
    assert_eq!(d.prompt, DEFAULT_PROMPT);

    let c = BenchRequest::from_params(&serde_json::json!({
        "prompt": "  hi  ", "runs": 99, "max_tokens": 4
    }));
    assert_eq!(c.prompt, "hi");
    assert_eq!(c.runs, MAX_RUNS);
    assert_eq!(c.max_tokens, MIN_MAX_TOKENS);
}

#[test]
fn request_picks_up_a_suite_and_halves_the_default_runs() {
    let r = BenchRequest::from_params(&serde_json::json!({ "suite": "  chat-v1  " }));
    assert_eq!(r.suite.as_deref(), Some("chat-v1"));
    assert_eq!(r.runs, DEFAULT_SUITE_RUNS);
    // The prompt/max_tokens fields stay at their defaults; the suite supplies both.
    assert_eq!(r.max_tokens, DEFAULT_MAX_TOKENS);

    let explicit = BenchRequest::from_params(&serde_json::json!({
        "suite": "coding-v1", "runs": 5
    }));
    assert_eq!(explicit.runs, 5);
    assert_eq!(
        BenchRequest::from_params(&serde_json::json!({ "suite": "coding-v1", "runs": 99 })).runs,
        MAX_RUNS
    );
}

#[test]
fn a_blank_suite_is_no_suite_at_all() {
    let blank = BenchRequest::from_params(&serde_json::json!({ "suite": "   " }));
    assert!(blank.suite.is_none());
    assert_eq!(blank.runs, DEFAULT_RUNS);
    assert!(
        BenchRequest::from_params(&serde_json::json!({ "suite": 7 }))
            .suite
            .is_none()
    );
    assert!(BenchRequest::default().suite.is_none());
}

// --- the pass plan and the pure aggregation ------------------------------

#[test]
fn without_a_suite_the_plan_is_the_single_request_prompt() {
    let req = BenchRequest {
        prompt: "hello".into(),
        max_tokens: 64,
        ..BenchRequest::default()
    };
    let steps = plan(&req).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].prompt_id, None);
    assert_eq!(steps[0].label, None);
    assert_eq!(steps[0].text, "hello");
    assert_eq!(steps[0].max_tokens, 64);
    assert_eq!(
        steps[0].opts,
        GenerationOptions::default(),
        "the quick test keeps sending exactly what it always sent"
    );
}

#[test]
fn a_suite_expands_into_its_prompts_at_the_suite_length() {
    let req = BenchRequest {
        suite: Some("coding-v1".into()),
        max_tokens: 16, // ignored — the suite decides
        ..BenchRequest::default()
    };
    let steps = plan(&req).unwrap();
    let suite = suites::find("coding-v1").unwrap();
    assert_eq!(steps.len(), 3);
    for (step, prompt) in steps.iter().zip(suite.prompts) {
        assert_eq!(step.prompt_id, Some(prompt.id));
        assert_eq!(step.text, prompt.text);
        assert_eq!(step.max_tokens, suite.max_tokens);
        assert_eq!(
            step.label.as_deref(),
            Some(format!("{} · {}", suite.id, prompt.title).as_str())
        );
        // Fixed length + fixed sampling + no prompt cache: two runs of the same
        // suite measure the same work (review I-1/I-2).
        assert_eq!(step.opts, GenerationOptions::fixed_length());
    }
}

#[test]
fn an_unknown_suite_is_a_clear_error() {
    let req = BenchRequest {
        suite: Some("chat-v9".into()),
        ..BenchRequest::default()
    };
    let err = plan(&req).unwrap_err().to_string();
    assert!(
        err.contains("unknown benchmark suite \u{201c}chat-v9\u{201d}"),
        "{err}"
    );
}

fn pass(
    prompt_id: Option<&'static str>,
    tokens: u64,
    gen_tps: f64,
    prompt_tps: Option<f64>,
) -> Pass {
    Pass {
        prompt_id,
        tokens,
        max_tokens: 128,
        gen_tps,
        prompt_tps,
    }
}

#[test]
fn aggregating_a_legacy_run_gives_means_and_no_detail() {
    let agg = aggregate(&[
        pass(None, 40, 50.0, Some(400.0)),
        pass(None, 40, 60.0, Some(500.0)),
        pass(None, 40, 70.0, Some(600.0)),
    ]);
    assert_eq!(agg.runs, 3);
    assert!((agg.gen_tps.unwrap() - 60.0).abs() < 1e-9);
    assert!((agg.prompt_tps.unwrap() - 500.0).abs() < 1e-9);
    assert!(agg.detail.is_empty());
    assert!((agg.stability - stability_score(&[50.0, 60.0, 70.0])).abs() < 1e-9);
}

#[test]
fn aggregating_a_suite_run_means_per_prompt_and_overall() {
    let agg = aggregate(&[
        pass(Some("a"), 100, 40.0, Some(400.0)),
        pass(Some("a"), 110, 60.0, Some(500.0)),
        pass(Some("b"), 41, 80.0, Some(600.0)),
        pass(Some("b"), 40, 100.0, Some(700.0)),
    ]);
    assert_eq!(agg.runs, 4);
    assert!((agg.gen_tps.unwrap() - 70.0).abs() < 1e-9, "{agg:?}");
    assert!((agg.prompt_tps.unwrap() - 550.0).abs() < 1e-9);

    // First-seen order, mean tokens rounded to whole tokens.
    let ids: Vec<&str> = agg.detail.iter().map(|d| d.prompt_id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
    assert_eq!(agg.detail[0].tokens, 105);
    assert_eq!(agg.detail[0].max_tokens, 128);
    assert!((agg.detail[0].gen_tps.unwrap() - 50.0).abs() < 1e-9);
    assert!((agg.detail[0].prompt_tps.unwrap() - 450.0).abs() < 1e-9);
    assert_eq!(agg.detail[1].tokens, 41, "40.5 rounds up");
    assert!((agg.detail[1].gen_tps.unwrap() - 90.0).abs() < 1e-9);
}

#[test]
fn a_missing_prompt_rate_is_skipped_rather_than_averaged_in_as_zero() {
    let agg = aggregate(&[
        pass(Some("a"), 40, 50.0, Some(400.0)),
        pass(Some("a"), 40, 50.0, None), // server reported no prefill rate
        pass(Some("b"), 40, 50.0, None),
    ]);
    assert!(
        (agg.prompt_tps.unwrap() - 400.0).abs() < 1e-9,
        "one usable sample, not 400/3: {agg:?}"
    );
    assert!((agg.detail[0].prompt_tps.unwrap() - 400.0).abs() < 1e-9);
    assert!(
        agg.detail[1].prompt_tps.is_none(),
        "no usable sample at all stays None"
    );
}

#[test]
fn suite_stability_is_the_mean_of_the_per_prompt_stabilities() {
    // Two prompts at very different speeds, each perfectly consistent with
    // itself: that is a stable machine, not an unstable one (review I-3).
    let agg = aggregate(&[
        pass(Some("fast"), 40, 100.0, Some(400.0)),
        pass(Some("fast"), 40, 100.0, Some(400.0)),
        pass(Some("slow"), 40, 20.0, Some(400.0)),
        pass(Some("slow"), 40, 20.0, Some(400.0)),
    ]);
    assert!((agg.stability - 1.0).abs() < 1e-9, "{agg:?}");
    // Pooling all four samples instead would have punished it badly.
    assert!(stability_score(&[100.0, 100.0, 20.0, 20.0]) < 0.5);

    // A prompt that really is erratic still drags the mean down.
    let jittery = aggregate(&[
        pass(Some("fast"), 40, 100.0, Some(400.0)),
        pass(Some("fast"), 40, 100.0, Some(400.0)),
        pass(Some("slow"), 40, 10.0, Some(400.0)),
        pass(Some("slow"), 40, 30.0, Some(400.0)),
    ]);
    assert!(jittery.stability < 0.8, "{jittery:?}");
}

#[test]
fn aggregating_a_single_pass_is_that_pass() {
    let agg = aggregate(&[pass(Some("only"), 77, 55.0, Some(410.0))]);
    assert_eq!(agg.runs, 1);
    assert!((agg.gen_tps.unwrap() - 55.0).abs() < 1e-9);
    assert!((agg.prompt_tps.unwrap() - 410.0).abs() < 1e-9);
    assert_eq!(agg.stability, 1.0, "one sample cannot disagree with itself");
    assert_eq!(agg.detail.len(), 1);
    assert_eq!(agg.detail[0].tokens, 77);
}

#[test]
fn aggregating_nothing_is_empty_rather_than_a_division_by_zero() {
    let agg = aggregate(&[]);
    assert_eq!(agg.runs, 0);
    assert!(agg.gen_tps.is_none());
    assert!(agg.prompt_tps.is_none());
    assert!(agg.detail.is_empty());
    assert_eq!(agg.stability, 1.0);
}

#[test]
fn an_empty_detail_is_stored_as_null_not_as_an_empty_array() {
    assert_eq!(detail_json(&[]), None);
    let one = detail_json(&[PromptResult {
        prompt_id: "a".into(),
        tokens: 40,
        max_tokens: 128,
        gen_tps: Some(50.0),
        prompt_tps: None,
    }])
    .unwrap();
    assert!(one.starts_with("[{"), "{one}");
    assert!(one.contains("\"max_tokens\":128"), "{one}");
}

// --- notes ---------------------------------------------------------------

#[test]
fn legacy_notes_keep_their_exact_wording() {
    let req = BenchRequest::default();
    assert_eq!(
        notes(&req, 3, Some(std::time::Duration::from_secs(1)), &[]),
        "cold load; 3 run(s) averaged"
    );
    assert_eq!(
        notes(&req, 3, None, &[]),
        "model already resident (load time not measured); 3 run(s)"
    );
}

#[test]
fn suite_notes_name_the_suite_and_flag_an_early_stop() {
    let req = BenchRequest {
        suite: Some("chat-v1".into()),
        ..BenchRequest::default()
    };
    let full = vec![PromptResult {
        prompt_id: "chat-v1-explain".into(),
        tokens: 256,
        max_tokens: 256,
        gen_tps: Some(50.0),
        prompt_tps: Some(400.0),
    }];
    assert_eq!(
        notes(&req, 6, None, &full),
        "model already resident (load time not measured); suite chat-v1, 6 pass(es) averaged"
    );

    let short = vec![
        full[0].clone(),
        PromptResult {
            prompt_id: "chat-v1-rewrite".into(),
            tokens: 90, // well under 0.9 × 256
            max_tokens: 256,
            gen_tps: Some(50.0),
            prompt_tps: Some(400.0),
        },
    ];
    let note = notes(&req, 6, None, &short);
    assert!(
        note.contains("stopped early on chat-v1-rewrite; tok/s not comparable"),
        "{note}"
    );
    assert!(
        !note.contains("chat-v1-explain;"),
        "only the short prompt is named: {note}"
    );
}
