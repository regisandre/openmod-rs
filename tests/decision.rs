//! How a workflow turns detector scores into an accept / review / reject
//! decision with explicit reasons.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::catalog::Catalog;
use openmod::decision::{Action, Score, evaluate};
use openmod::workflow::{Threshold, Workflow};
use std::collections::BTreeMap;

fn workflow(rules: &[(&str, Option<f32>, Option<f32>)]) -> Workflow {
    let mut map = BTreeMap::new();
    for (id, reject, review) in rules {
        map.insert(
            id.to_string(),
            Threshold {
                reject: *reject,
                review: *review,
            },
        );
    }
    Workflow { rules: map }
}

fn score(category: &str, value: f32, source: &str) -> Score {
    Score {
        category: category.to_string(),
        score: value,
        source: source.to_string(),
    }
}

#[test]
fn decision_follows_the_workflow_thresholds() {
    let wf = workflow(&[
        ("nudity", Some(0.8), Some(0.5)),
        ("weapon", Some(0.3), None),
        ("alcohol", None, Some(0.4)),
    ]);
    let catalog = Catalog::builtin();

    // (case, scores, expected action, expected reject ids, expected review ids)
    let cases: Vec<(&str, Vec<Score>, Action, Vec<&str>, Vec<&str>)> = vec![
        (
            "nothing above any threshold",
            vec![score("nudity", 0.2, "a"), score("weapon", 0.1, "b")],
            Action::Accept,
            vec![],
            vec![],
        ),
        (
            "reject threshold reached",
            vec![score("nudity", 0.9, "a")],
            Action::Reject,
            vec!["nudity"],
            vec![],
        ),
        (
            "reject threshold is inclusive",
            vec![score("nudity", 0.8, "a")],
            Action::Reject,
            vec!["nudity"],
            vec![],
        ),
        (
            "between review and reject",
            vec![score("nudity", 0.6, "a")],
            Action::Review,
            vec![],
            vec!["nudity"],
        ),
        (
            "review-only category never rejects",
            vec![score("alcohol", 0.99, "a")],
            Action::Review,
            vec![],
            vec!["alcohol"],
        ),
        (
            "category absent from the workflow is ignored",
            vec![score("gore", 1.0, "a")],
            Action::Accept,
            vec![],
            vec![],
        ),
        (
            "reject wins over review",
            vec![score("alcohol", 0.9, "a"), score("weapon", 0.5, "b")],
            Action::Reject,
            vec!["weapon"],
            vec!["alcohol"],
        ),
        (
            "two detectors on one category keep the highest",
            vec![score("nudity", 0.3, "a"), score("nudity", 0.85, "b")],
            Action::Reject,
            vec!["nudity"],
            vec![],
        ),
        (
            "reasons sorted by score, highest first",
            vec![score("weapon", 0.4, "b"), score("nudity", 0.95, "a")],
            Action::Reject,
            vec!["nudity", "weapon"],
            vec![],
        ),
    ];

    for (case, scores, action, reject_ids, review_ids) in cases {
        let summary = evaluate(&wf, &scores, &catalog);
        let got_reject: Vec<&str> = summary
            .reject_reason
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        let got_review: Vec<&str> = summary
            .review_reason
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(summary.action, action, "action: {case}");
        assert_eq!(got_reject, reject_ids, "reject reasons: {case}");
        assert_eq!(got_review, review_ids, "review reasons: {case}");
    }
}

#[test]
fn a_reason_tells_the_developer_what_matched_and_why() {
    let wf = workflow(&[("nudity", Some(0.8), Some(0.5))]);
    let summary = evaluate(
        &wf,
        &[score("nudity", 0.91, "marqo_nsfw")],
        &Catalog::builtin(),
    );
    let reason = &summary.reject_reason[0];

    let checks: Vec<(&str, bool)> = vec![
        ("stable machine id", reason.id == "nudity"),
        ("human title", reason.text == "Nudity"),
        (
            "full sentence for end users",
            reason.message.ends_with('.') && reason.message.len() > 10,
        ),
        ("score that matched", (reason.score - 0.91).abs() < 1e-6),
        ("threshold that was crossed", reason.threshold == 0.8),
        ("detector that produced it", reason.source == "marqo_nsfw"),
        (
            "reject_prob is the top rejected score",
            (summary.reject_prob - 0.91).abs() < 1e-6,
        ),
    ];
    for (case, ok) in checks {
        assert!(ok, "{case}: {reason:?}");
    }
}

#[test]
fn a_second_opinion_is_averaged_only_where_both_models_speak() {
    let first = vec![
        score("hate", 0.9, "gli"),
        score("violence", 0.6, "gli"),
        score("violence", 0.2, "gli"),
    ];
    let second = vec![
        score("violence", 0.0, "qwen"),
        score("jailbreak", 0.7, "qwen"),
    ];
    let merged = openmod::decision::average_opinions(first, second);
    let get = |c: &str| {
        merged
            .iter()
            .find(|s| s.category == c)
            .map(|s| (s.score, s.source.as_str()))
    };

    // (case, category, expected score and source)
    let cases = [
        (
            "only the first model scores it: kept",
            "hate",
            Some((0.9, "gli")),
        ),
        (
            "both score it: mean of each model's best",
            "violence",
            Some((0.3, "gli+qwen")),
        ),
        (
            "only the second model scores it: added",
            "jailbreak",
            Some((0.7, "qwen")),
        ),
        ("nobody scores it", "sexual", None),
    ];
    for (case, category, expected) in cases {
        let got = get(category);
        match (got, expected) {
            (Some((s, src)), Some((e, esrc))) => {
                assert!((s - e).abs() < 1e-6, "{case}: score {s} != {e}");
                assert_eq!(src, esrc, "{case}: source");
            }
            (g, e) => assert_eq!(g.is_some(), e.is_some(), "{case}"),
        }
    }
    assert_eq!(merged.len(), 3, "one score per category");
}

#[test]
fn a_category_several_models_score_combines_as_configured() {
    use openmod::config::Combine;
    use openmod::decision::{best_scores, combine_scores};

    let rules: BTreeMap<String, Combine> = [("nudity".to_string(), Combine::Mean)].into();
    // (case, scores, category, expected score and source)
    let cases: Vec<(&str, Vec<Score>, &str, (f32, &str))> = vec![
        (
            "mean: each model's best score, averaged",
            vec![
                score("nudity", 0.9, "marqo"),
                score("nudity", 0.3, "owen"),
                score("nudity", 0.5, "owen"),
            ],
            "nudity",
            (0.7, "marqo+owen"),
        ),
        (
            "mean with a single model: its own score",
            vec![score("nudity", 0.9, "marqo")],
            "nudity",
            (0.9, "marqo"),
        ),
        (
            "any other category: the highest score",
            vec![score("gore", 0.8, "owen"), score("gore", 0.2, "siglip")],
            "gore",
            (0.8, "owen"),
        ),
    ];
    for (case, scores, category, (expected, source)) in cases {
        let best = best_scores(&combine_scores(scores, &rules));
        let got = best.get(category).expect(case);
        assert!(
            (got.score - expected).abs() < 1e-6,
            "{case}: score {} != {expected}",
            got.score
        );
        assert_eq!(got.source, source, "{case}: source");
    }
}

#[test]
fn a_text_category_never_outscores_the_overall_verdict() {
    let rules_hit = score("pii_phone", 1.0, "rules");
    // (case, scores, category to read, expected score)
    let cases: Vec<(&str, Vec<Score>, &str, f32)> = vec![
        (
            "category above the verdict is capped",
            vec![score("text_unsafe", 0.3, "m"), score("hate", 0.99, "m")],
            "hate",
            0.3,
        ),
        (
            "category below the verdict is kept",
            vec![score("text_unsafe", 0.9, "m"), score("hate", 0.7, "m")],
            "hate",
            0.7,
        ),
        (
            "the verdict itself is kept",
            vec![score("text_unsafe", 0.3, "m"), score("hate", 0.99, "m")],
            "text_unsafe",
            0.3,
        ),
        (
            "rules are never capped",
            vec![score("text_unsafe", 0.0, "m"), rules_hit.clone()],
            "pii_phone",
            1.0,
        ),
        (
            "no verdict, nothing capped",
            vec![score("hate", 0.99, "m")],
            "hate",
            0.99,
        ),
    ];
    for (case, scores, category, expected) in cases {
        let capped = openmod::decision::cap_to_verdict(scores, "text_unsafe");
        let got = capped
            .iter()
            .find(|s| s.category == category)
            .map(|s| s.score);
        assert_eq!(got, Some(expected), "{case}");
    }
}

#[test]
fn unknown_categories_fall_back_to_a_generic_reason() {
    let wf = workflow(&[("custom_logo", Some(0.5), None)]);
    let summary = evaluate(
        &wf,
        &[score("custom_logo", 0.7, "siglip")],
        &Catalog::builtin(),
    );
    let reason = &summary.reject_reason[0];
    assert_eq!(reason.id, "custom_logo");
    assert_eq!(reason.text, "Custom logo", "title derived from the id");
    assert!(!reason.message.is_empty());
}

#[test]
fn a_workflow_answers_only_when_at_least_one_of_its_rules_can_be_checked() {
    use openmod::workflow::Unavailable;
    let available: std::collections::BTreeSet<String> = ["nudity", "pii_email"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // (case, rules, expected unchecked categories or the refusal)
    let cases: Vec<(&str, Workflow, Result<Vec<&str>, Unavailable>)> = vec![
        (
            "every rule checked",
            workflow(&[("nudity", Some(0.8), None)]),
            Ok(vec![]),
        ),
        (
            "one rule unchecked",
            workflow(&[("nudity", Some(0.8), None), ("weapon", Some(0.6), None)]),
            Ok(vec!["weapon"]),
        ),
        (
            "no rule checkable",
            workflow(&[("weapon", Some(0.6), None), ("gore", Some(0.6), None)]),
            Err(Unavailable::NothingCheckable(vec![
                "gore".into(),
                "weapon".into(),
            ])),
        ),
        ("no rule at all", workflow(&[]), Err(Unavailable::NoRules)),
    ];
    for (case, wf, expected) in cases {
        let expected = expected.map(|v| v.into_iter().map(String::from).collect::<Vec<_>>());
        assert_eq!(wf.plan(&available), expected, "{case}");
    }
}
