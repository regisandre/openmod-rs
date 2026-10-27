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

const ZERO_SHOT_NEGATIVES: &str = r#"
[models.zs]
kind = "image_zero_shot"
vision = "v.onnx"
text = "t.onnx"
tokenizer = "tok.json"
size = 224
logit_scale = 4.7
logit_bias = -12.9
[models.zs.concepts]
violence = { shift = 5.9, prompts = ["people fighting"], negatives = ["people dancing"] }
"#;

const ZERO_SHOT_MISSPELLED: &str = r#"
[models.zs]
kind = "image_zero_shot"
vision = "v.onnx"
text = "t.onnx"
tokenizer = "tok.json"
size = 224
logit_scale = 4.7
logit_bias = -12.9
[models.zs.concepts]
violence = { shift = 5.9, prompts = ["people fighting"], negative = ["people dancing"] }
"#;

const READERS: &str = r#"
[models.adult_sites]
kind = "domain_list"
paths = ["lists/porn.txt"]
public_suffixes = "lists/psl.dat"
category = "link_adult"

[models.qr]
kind = "qr_reader"

[models.ocr]
kind = "image_ocr"
detector = "d.onnx"
recognizer = "r.onnx"
dictionary = "r.yml"
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
    let no_prompt = ZERO_SHOT_NEGATIVES.replace("[\"people fighting\"]", "[]");
    let readers_and_text = format!("{READERS}\n{TEXT_MODELS}");
    let list_without_file = READERS.replace("paths = [\"lists/porn.txt\"]", "paths = []");
    let allowed_url = READERS.replace(
        "category = \"link_adult\"",
        "category = \"link_adult\"\nallow = [\"https://night.events/\"]",
    );
    let tiny_ocr = READERS.replace(
        "dictionary = \"r.yml\"",
        "dictionary = \"r.yml\"\nmax_side = 10",
    );
    let keeps_encoder = ZERO_SHOT_NEGATIVES.replace(
        "[models.zs.concepts]",
        "keep_text_encoder = true\n[models.zs.concepts]",
    );
    let misspelled_encoder = ZERO_SHOT_NEGATIVES.replace(
        "[models.zs.concepts]",
        "keep_text_encoders = true\n[models.zs.concepts]",
    );
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
        (
            "a category combined as a mean",
            MODELS,
            "[workflows.default.image]\nnudity = { reject = 0.8 }\n[combine]\nnudity = \"mean\"",
            None,
        ),
        (
            "combining a category no model scores",
            MODELS,
            "[workflows.default.image]\nnudity = { reject = 0.8 }\n[combine]\nnudty = \"mean\"",
            Some("`nudty`"),
        ),
        (
            "an unknown way to combine",
            MODELS,
            "[workflows.default.image]\nnudity = { reject = 0.8 }\n[combine]\nnudity = \"median\"",
            Some("median"),
        ),
        (
            "a concept with negative prompts",
            ZERO_SHOT_NEGATIVES,
            "[workflows.default.image]\nviolence = { reject = 0.8 }",
            None,
        ),
        (
            "a concept with negatives but no prompt",
            &no_prompt,
            "[workflows.default.image]\nviolence = { reject = 0.8 }",
            Some("no prompt"),
        ),
        (
            "a misspelled concept setting is refused",
            ZERO_SHOT_MISSPELLED,
            "[workflows.default.image]\nviolence = { reject = 0.8 }",
            Some("negative"),
        ),
        (
            "a zero-shot model that keeps its text encoder",
            &keeps_encoder,
            "[workflows.default.image]\nviolence = { reject = 0.8 }",
            None,
        ),
        (
            "a misspelled setting of the same model is refused",
            &misspelled_encoder,
            "[workflows.default.image]\nviolence = { reject = 0.8 }",
            Some("unknown setting `keep_text_encoders`"),
        ),
        (
            "text and QR codes read out of images",
            READERS,
            "[workflows.default.image]\ntext_link_adult = { reject = 1.0 }\nqr_link_adult = { reject = 1.0 }\ntext_extremism = { reject = 1.0 }\nqr_code = { review = 1.0 }\n[workflows.default.text]\nlink_adult = { reject = 1.0 }",
            None,
        ),
        (
            "a domain list scores texts, not images",
            READERS,
            "[workflows.default.image]\nlink_adult = { reject = 1.0 }",
            Some("`link_adult`"),
        ),
        (
            "no OCR, no text read out of images",
            MODELS,
            "[workflows.default.image]\ntext_link = { reject = 1.0 }",
            Some("`text_link`"),
        ),
        (
            "the text of an image goes through the text models",
            &readers_and_text,
            "[workflows.default.image]\ntext_unsafe = { reject = 0.8 }",
            None,
        ),
        (
            "a QR code does not",
            &readers_and_text,
            "[workflows.default.image]\nqr_text_unsafe = { reject = 0.8 }",
            Some("`qr_text_unsafe`"),
        ),
        (
            "a domain list without a file",
            &list_without_file,
            "[workflows.default.text]\nlink_adult = { reject = 1.0 }",
            Some("`paths`"),
        ),
        (
            "a domain list allowing a URL instead of a domain",
            &allowed_url,
            "[workflows.default.text]\nlink_adult = { reject = 1.0 }",
            Some("`allow`: `https://night.events/` is not a domain name"),
        ),
        (
            "an OCR detector shown a thumbnail",
            &tiny_ocr,
            "[workflows.default.image]\ntext_link = { reject = 1.0 }",
            Some("`max_side`"),
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
fn a_category_read_out_of_an_image_needs_its_reader_and_the_tool_behind_it() {
    let config = parse(&format!(
        "{READERS}
{TEXT_MODELS}"
    ))
    .unwrap();
    // (case, category, for images, tools that score it)
    let cases: Vec<(&str, &str, bool, Vec<&str>)> = vec![
        (
            "a domain list read in the text of an image",
            "text_link_adult",
            true,
            vec!["ocr+adult_sites"],
        ),
        (
            "the same list read in a QR code",
            "qr_link_adult",
            true,
            vec!["qr+adult_sites"],
        ),
        ("a rule read in a QR code", "qr_link", true, vec!["qr"]),
        (
            "the text models read in the text of an image",
            "text_unsafe",
            true,
            vec!["ocr+gliguard", "ocr+qwen"],
        ),
        ("any QR code at all", "qr_code", true, vec!["qr"]),
        (
            "the list itself, for texts",
            "link_adult",
            false,
            vec!["adult_sites"],
        ),
    ];
    for (case, category, image, expected) in cases {
        assert_eq!(config.producers(category, image), expected, "{case}");
    }
    let off = parse(&format!(
        "{}\n{TEXT_MODELS}",
        READERS.replace(
            "category = \"link_adult\"",
            "category = \"link_adult\"\nenabled = false"
        )
    ))
    .unwrap();
    // (case, category, for images, known, available)
    let cases: Vec<(&str, &str, bool, bool, bool)> = vec![
        (
            "a disabled list, read in images",
            "text_link_adult",
            true,
            true,
            false,
        ),
        (
            "a disabled list, for texts",
            "link_adult",
            false,
            true,
            false,
        ),
        ("the rules still read images", "text_link", true, true, true),
    ];
    for (case, category, image, known, available) in cases {
        assert_eq!(
            off.known_categories(image).contains(category),
            known,
            "{case}: known"
        );
        assert_eq!(
            off.available_categories(image).contains(category),
            available,
            "{case}: available"
        );
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
fn the_default_config_is_valid_ships_every_builtin_tool_and_words_every_category() {
    let config = parse(DEFAULT_CONFIG).unwrap();
    let catalog = openmod::catalog::Catalog::builtin();
    let categories = config
        .known_categories(true)
        .into_iter()
        .chain(config.known_categories(false));
    for category in categories {
        let label = catalog.label(&category);
        assert!(
            !label.message.starts_with("This content was flagged for"),
            "`{category}` falls back to the generic message"
        );
    }
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
