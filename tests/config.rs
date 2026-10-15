//! What a configuration file may and may not say.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::config::{Config, DEFAULT_CONFIG};
use openmod::tools::TOOLS;
use std::collections::BTreeSet;
use std::path::Path;

const MODELS: &str = r#"
[models.nsfw]
kind = "image_classifier"
path = "m.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }

[models.zs]
kind = "image_zero_shot"
vision = "v.onnx"
text = "t.onnx"
tokenizer = "tok.json"
size = 224
logit_scale = 4.7
logit_bias = -12.9
[models.zs.concepts]
weapon = ["a gun"]
alcohol = { shift = 6.5, prompts = ["a beer"] }
"#;

const DISABLED_NSFW: &str = r#"
[models.nsfw]
kind = "image_classifier"
enabled = false
path = "m.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }
"#;

const MISSPELLED_SETTING: &str = r#"
[models.nsfw]
kind = "image_classifier"
enabeld = false
path = "m.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }
"#;

const TEXT_MODELS: &str = r#"
[models.gliguard]
kind = "text_classifier"
path = "g.onnx"
tokenizer = "g.json"
[[models.gliguard.tasks]]
name = "prompt_safety"
labels = ["safe", "unsafe"]
categories = { unsafe = "text_unsafe" }

[models.qwen]
kind = "text_guard"
path = "q.onnx"
tokenizer = "q.json"
second_opinion = { category = "text_unsafe", low = 0.2, high = 1.0 }

[workflows.default.text]
text_unsafe = { reject = 0.8 }
"#;

fn parse(text: &str) -> Result<Config, String> {
    Config::parse(text, Path::new(".")).map_err(|e| format!("{e:#}"))
}

#[test]
fn config_validation_refuses_what_would_silently_disable_a_rule() {
    // (case, models, workflows, expected error fragment or None when valid)
    let cases: Vec<(&str, &str, &str, Option<&str>)> = vec![
        (
            "valid image and text rules",
            MODELS,
            "[workflows.default.image]\nnudity = { reject = 0.8 }\nweapon = { review = 0.5 }\nalcohol = { review = 0.5 }\n[workflows.default.text]\npii_email = { review = 1.0 }",
            None,
        ),
        (
            "a default workflow is required",
            MODELS,
            "[workflows.other.image]\nnudity = { reject = 0.8 }",
            Some("`default`"),
        ),
        (
            "typo in an image category",
            MODELS,
            "[workflows.default.image]\nnudty = { reject = 0.8 }",
            Some("`nudty`"),
        ),
        (
            "text category used for images",
            MODELS,
            "[workflows.default.image]\npii_email = { reject = 1.0 }",
            Some("`pii_email`"),
        ),
        (
            "image category used for text",
            MODELS,
            "[workflows.default.text]\nnudity = { reject = 0.5 }",
            Some("`nudity`"),
        ),
        (
            "threshold above 1",
            MODELS,
            "[workflows.default.image]\nnudity = { reject = 1.5 }",
            Some("outside [0, 1]"),
        ),
        (
            "negative threshold",
            MODELS,
            "[workflows.default.image]\nnudity = { review = -0.1 }",
            Some("outside [0, 1]"),
        ),
        (
            "rule without any threshold",
            MODELS,
            "[workflows.default.image]\nnudity = {}",
            Some("neither"),
        ),
        (
            "unknown threshold key",
            MODELS,
            "[workflows.default.image]\nnudity = { block = 0.5 }",
            Some("unknown field"),
        ),
        (
            "a disabled model keeps its categories valid",
            DISABLED_NSFW,
            "[workflows.default.image]\nnudity = { reject = 0.8 }",
            None,
        ),
        (
            "a misspelled model setting is refused",
            MISSPELLED_SETTING,
            "[workflows.default.image]\nnudity = { reject = 0.8 }",
            Some("unknown setting `enabeld`"),
        ),
    ];
    for (case, models, workflows, expected) in cases {
        let result = parse(&format!("{models}\n{workflows}"));
        match expected {
            None => assert!(result.is_ok(), "{case}: {result:?}"),
            Some(fragment) => {
                let err = result.expect_err(case);
                assert!(err.contains(fragment), "{case}: {err}");
            }
        }
    }
}

#[test]
fn a_second_opinion_without_its_first_pass_runs_on_every_text() {
    let config = parse(TEXT_MODELS).unwrap();
    // (case, ready text models, expected (model, gives a second opinion))
    let cases: Vec<(&str, Vec<&str>, Vec<(&str, bool)>)> = vec![
        (
            "both ready",
            vec!["gliguard", "qwen"],
            vec![("gliguard", false), ("qwen", true)],
        ),
        ("first pass not ready", vec!["qwen"], vec![("qwen", false)]),
        (
            "second opinion not ready",
            vec!["gliguard"],
            vec![("gliguard", false)],
        ),
    ];
    for (case, ready, expected) in cases {
        let ready: BTreeSet<String> = ready.into_iter().map(String::from).collect();
        let got: Vec<(String, bool)> = config
            .text_plan(&ready)
            .into_iter()
            .map(|(name, so)| (name, so.is_some()))
            .collect();
        let expected: Vec<(String, bool)> = expected
            .into_iter()
            .map(|(n, s)| (n.to_string(), s))
            .collect();
        assert_eq!(got, expected, "{case}");
    }
}

#[test]
fn the_default_config_is_valid_and_ships_every_builtin_tool() {
    let config = parse(DEFAULT_CONFIG).unwrap();
    for tool in TOOLS {
        let entry = config.models.get(tool.id);
        assert!(
            entry.is_some(),
            "the default config has no `[models.{}]` block",
            tool.id
        );
        for file in tool.files {
            let named = entry
                .unwrap()
                .spec
                .files()
                .iter()
                .any(|p| p.ends_with(file.name));
            assert!(named, "`{}` does not point at {}", tool.id, file.name);
        }
    }
}
