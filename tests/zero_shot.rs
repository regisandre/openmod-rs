//! How a zero-shot category turns prompt similarities into a score.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use image::{ImageFormat, Rgb, RgbImage};
use openmod::config::{ConceptSpec, Config, ModelSpec};
use openmod::engine::{CheckError, Engine, ToolStatus};
use openmod::models::onnx::{DimensionMismatch, concept_score, score_from_embeddings};
use openmod::workflow::{CompiledWorkflow, WorkflowSpec};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

// SigLIP base: exp(logit_scale) and logit_bias from openmod.toml.
const SCALE: f32 = 117.33;
const BIAS: f32 = -12.93;

/// A unit vector whose similarity to the image `[1, 0, 0]` is `cos`.
fn at(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt(), 0.0]
}

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

#[test]
fn a_concept_scores_from_its_embeddings() {
    let image = vec![1.0, 0.0, 0.0];
    // (case, similarity of each prompt, of each negative, shift, expected score).
    // The best prompt and the best negative are never the first of their list.
    let cases: Vec<(&str, Vec<f32>, Vec<f32>, f32, f32)> = vec![
        (
            "no negatives: SigLIP's probability for the best prompt",
            vec![0.05, 0.15, 0.10],
            vec![],
            0.0,
            0.9907,
        ),
        (
            "the shift moves the decision boundary",
            vec![0.05, 0.15, 0.10],
            vec![],
            -4.67,
            0.5,
        ),
        (
            "negatives far below the best prompt change almost nothing",
            vec![0.05, 0.15, 0.10],
            vec![0.02, 0.05, 0.03],
            0.0,
            0.9907,
        ),
        (
            "a negative as close as the best prompt halves the score",
            vec![0.05, 0.15, 0.10],
            vec![0.02, 0.15, 0.10],
            0.0,
            0.4953,
        ),
        (
            "a negative closer than the best prompt wins",
            vec![0.05, 0.15, 0.10],
            vec![0.02, 0.20, 0.10],
            0.0,
            0.0028,
        ),
        (
            "the shift moves the prompt's probability, not the negative's",
            vec![0.05, 0.15, 0.10],
            vec![0.02, 0.15, 0.10],
            -4.67,
            0.25,
        ),
        ("no prompt, nothing to match", vec![], vec![0.05], 0.0, 0.0),
        (
            "an unrelated image stays near zero",
            vec![0.02],
            vec![0.01],
            0.0,
            0.0,
        ),
    ];
    let mut failures = Vec::new();
    for (case, prompts, negatives, shift, expected) in cases {
        let prompts: Vec<Vec<f32>> = prompts.into_iter().map(at).collect();
        let negatives: Vec<Vec<f32>> = negatives.into_iter().map(at).collect();
        match score_from_embeddings(&image, &prompts, &negatives, SCALE, BIAS, shift) {
            Ok(got) if (got - expected).abs() < 0.0005 => {}
            got => failures.push(format!("{case}: got {got:?}, expected {expected}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn an_embedding_of_another_size_is_refused_not_truncated() {
    let image = vec![1.0, 0.0, 0.0];
    let mismatch = |found| Err(DimensionMismatch { expected: 3, found });
    // (case, prompts, negatives, expected)
    let cases: Vec<(
        &str,
        Vec<Vec<f32>>,
        Vec<Vec<f32>>,
        Result<(), DimensionMismatch>,
    )> = vec![
        (
            "every embedding has the size of the image's",
            vec![at(0.1)],
            vec![at(0.05)],
            Ok(()),
        ),
        (
            "a prompt with fewer values",
            vec![at(0.1), vec![0.5, 0.5]],
            vec![],
            mismatch(2),
        ),
        (
            "a negative with more values",
            vec![at(0.1)],
            vec![vec![0.0; 4]],
            mismatch(4),
        ),
    ];
    let mut failures = Vec::new();
    for (case, prompts, negatives, expected) in cases {
        let got = score_from_embeddings(&image, &prompts, &negatives, SCALE, BIAS, 0.0);
        if got.map(|_| ()) != expected {
            failures.push(format!("{case}: got {got:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Only SigLIP, from the model files in `models/`, with the given concepts of
/// its own and the default image rules.
fn siglip(concepts: &str, keep_text_encoder: bool, image_rules: &str) -> Engine {
    let raw = format!(
        r#"
[models.siglip]
kind = "image_zero_shot"
vision = "models/siglip-vision-int8.onnx"
text = "models/siglip-text-int8.onnx"
tokenizer = "models/siglip-tokenizer.json"
size = 224
resize = "stretch"
filter = "bicubic"
pixel_range = "unit"
logit_scale = 4.7649970054626465
logit_bias = -12.9324369430542
keep_text_encoder = {keep_text_encoder}
[models.siglip.concepts]
{concepts}

[workflows.default.image]
{image_rules}
"#
    );
    let engine = Engine::load(Config::parse(&raw, Path::new(".")).unwrap(), None);
    assert_eq!(
        engine.status["siglip"],
        ToolStatus::Ready,
        "run `openmod fetch` first: {:?}",
        engine.problems()
    );
    engine
}

/// A picture of `kind`, as PNG bytes: something for SigLIP to look at.
fn picture(kind: &str) -> Vec<u8> {
    let img = RgbImage::from_fn(256, 256, |x, y| match kind {
        "dusk" => {
            // A sky from blue to orange, with warm round lights in it.
            let glow = ((x as f32 - 128.0).powi(2) + (y as f32 - 150.0).powi(2)).sqrt() < 40.0;
            if glow {
                Rgb([255, 190, 60])
            } else {
                Rgb([(y / 2) as u8, (40 + y / 4) as u8, (150 - y / 2) as u8])
            }
        }
        "stripes" => Rgb([if (x / 16) % 2 == 0 { 20 } else { 235 }; 3]),
        _ => {
            let n = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
            Rgb([n as u8, (n >> 8) as u8, (n >> 16) as u8])
        }
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

#[test]
#[ignore = "needs the model files in models/ (run `openmod fetch`)"]
fn compiled_concept_matches_config_concept() {
    let lantern = r#"lantern = { shift = 11.5, prompts = ["a photo of a paper lantern", "a photo of glowing lanterns"], negatives = ["a photo of a street lamp"] }"#;
    // The concept written in the configuration, and the very same one
    // compiled at run time by an engine that keeps its text encoder.
    let configured = siglip(lantern, false, "lantern = { review = 0.5 }");
    let ModelSpec::ImageZeroShot { concepts, .. } = &configured.config.models["siglip"].spec else {
        panic!("siglip is a zero-shot model");
    };
    let at_run_time = siglip("", true, "");
    let compiled = at_run_time
        .compile_workflow(&configured.config.workflows["default"], concepts)
        .unwrap_or_else(|problems| panic!("{problems:?}"));

    let mut scores = Vec::new();
    let mut failures = Vec::new();
    for kind in ["dusk", "stripes", "noise"] {
        let image = picture(kind);
        let from_config = configured.check_image(&image, "default").unwrap();
        let from_run_time = at_run_time
            .check_image_with(&image, "default", &compiled)
            .unwrap();
        let (a, b) = (
            from_config.scores["lantern"],
            from_run_time.scores["lantern"],
        );
        scores.push(a);
        if a != b {
            failures.push(format!("{kind}: configured {a}, compiled {b}"));
        }
    }
    println!("scores of the configured concept: {scores:?}");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(
        scores.iter().any(|s| *s > 0.0 && *s < 1.0) && scores.windows(2).any(|w| w[0] != w[1]),
        "the pictures must not all score the same, or 0, or 1: {scores:?}"
    );
}

#[test]
#[ignore = "needs the model files in models/ (run `openmod fetch`)"]
fn a_zero_shot_model_that_freed_its_text_encoder_cannot_compile_a_concept() {
    let engine = siglip("", false, "");
    let spec: WorkflowSpec =
        serde_json::from_str(r#"{ "image": { "fireworks": { "review": 0.5 } } }"#).unwrap();
    let concepts: BTreeMap<String, ConceptSpec> =
        serde_json::from_str(r#"{ "fireworks": ["a photo of fireworks"] }"#).unwrap();
    match engine.compile_workflow(&spec, &concepts) {
        Err(problems) if problems.len() == 1 => assert!(
            problems[0]
                .message
                .contains("set `keep_text_encoder = true` in `[models.siglip]`"),
            "{problems:?}"
        ),
        other => panic!("want one problem about keep_text_encoder, got {other:?}"),
    }
}

#[test]
#[ignore = "needs the model files in models/ (run `openmod fetch`)"]
fn a_workflow_compiled_by_another_model_is_refused_at_check_time() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    let engine = siglip("", true, "");
    let spec: WorkflowSpec =
        serde_json::from_str(r#"{ "image": { "fireworks": { "review": 0.5 } } }"#).unwrap();
    let concepts: BTreeMap<String, ConceptSpec> =
        serde_json::from_str(r#"{ "fireworks": ["a photo of fireworks"] }"#).unwrap();
    let compiled = engine.compile_workflow(&spec, &concepts).unwrap();
    let image = picture("dusk");
    assert!(
        engine.check_image_with(&image, "w", &compiled).is_ok(),
        "the workflow compiled by this very model checks"
    );

    // Real embeddings come back from their stored form exactly.
    let mut stored = serde_json::to_value(&compiled).unwrap();
    let again: CompiledWorkflow = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(again, compiled, "a stored workflow reads back the same");

    // The same workflow, stored with embeddings of 2 values: the size another
    // model would have given them.
    stored["concepts"][0]["dimension"] = serde_json::json!(2);
    for set in ["prompts", "negatives"] {
        for prompt in stored["concepts"][0][set].as_array_mut().unwrap() {
            let bytes = STANDARD
                .decode(prompt["embedding"].as_str().unwrap())
                .unwrap();
            prompt["embedding"] = serde_json::json!(STANDARD.encode(&bytes[..8]));
        }
    }
    let smaller: CompiledWorkflow = serde_json::from_value(stored).unwrap();
    match engine.check_image_with(&image, "w", &smaller) {
        Err(CheckError::Incompatible(message)) => assert!(
            message.contains("hold 2 values, `siglip` gives 768"),
            "{message}"
        ),
        other => panic!("want Incompatible, got {other:?}"),
    }
}
