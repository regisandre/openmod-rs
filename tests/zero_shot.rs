//! How a zero-shot category turns prompt similarities into a score.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::models::onnx::concept_score;

// SigLIP base: exp(logit_scale) and logit_bias from openmod.toml.
const SCALE: f32 = 117.33;
const BIAS: f32 = -12.93;

#[test]
fn a_negative_prompt_pulls_down_an_image_that_looks_more_like_it() {
    // (case, best prompt similarity, best negative similarity, shift, expected score)
    let cases: Vec<(&str, f32, Option<f32>, f32, f32)> = vec![
        (
            "no negatives: the best prompt's own probability",
            0.15,
            None,
            0.0,
            0.9907,
        ),
        (
            "the shift moves the decision boundary",
            0.15,
            None,
            -4.67,
            0.5,
        ),
        (
            "a negative far below the prompt changes almost nothing",
            0.15,
            Some(0.05),
            0.0,
            0.9907,
        ),
        (
            "a tie between prompt and negative halves the score",
            0.15,
            Some(0.15),
            0.0,
            0.4953,
        ),
        (
            "a negative that matches better wins",
            0.15,
            Some(0.20),
            0.0,
            0.0028,
        ),
        (
            "an unrelated image stays near zero",
            0.02,
            Some(0.01),
            0.0,
            0.0,
        ),
    ];
    for (case, pos, neg, shift, expected) in cases {
        let got = concept_score(pos, neg, SCALE, BIAS, shift);
        assert!(
            (got - expected).abs() < 0.002,
            "{case}: got {got}, expected {expected}"
        );
    }
}
