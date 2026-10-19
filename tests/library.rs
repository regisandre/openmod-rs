//! Using openmod as a library: workflows that live outside `openmod.toml`,
//! compiled at run time, checked against, stored and read back.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use image::{ImageFormat, Rgb, RgbImage};
use openmod::catalog;
use openmod::config::{ConceptSpec, Config, DEFAULT_CONFIG, ModelSpec};
use openmod::decision::Action;
use openmod::engine::{CheckError, Engine, Report};
use openmod::tools::Kind;
use openmod::workflow::{CompiledWorkflow, WorkflowSpec};
use qrcode::{Color, QrCode};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

/// A server with no model file to download: the QR reader, a one-site adult
/// list, the text rules, and a disabled classifier whose rule stays unchecked.
fn engine(name: &str) -> Engine {
    let dir = std::env::temp_dir().join(format!("openmod-library-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lists")).unwrap();
    std::fs::write(dir.join("lists/porn.txt"), "0.0.0.0 adult-site.com\n").unwrap();
    std::fs::write(dir.join("lists/psl.dat"), "com\n").unwrap();
    std::fs::write(
        dir.join("openmod.toml"),
        r#"
[models.qr]
kind = "qr_reader"

[models.adult_sites]
kind = "domain_list"
paths = ["lists/porn.txt"]
public_suffixes = "lists/psl.dat"
category = "link_adult"

[models.nsfw]
kind = "image_classifier"
enabled = false
path = "models/nsfw.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }

[workflows.default.image]
nudity = { reject = 0.8, review = 0.5 }
qr_link_adult = { reject = 1.0 }
qr_code = { review = 1.0 }

[workflows.default.text]
pii_iban = { reject = 1.0 }
pii_phone = { review = 1.0 }
"#,
    )
    .unwrap();
    Engine::load(Config::load(&dir.join("openmod.toml")).unwrap(), None)
}

/// A photo-like background with, if given, a QR code of `payload` on it.
fn png(payload: Option<&str>) -> Vec<u8> {
    let (w, h) = (900, 700);
    let mut img = RgbImage::from_fn(w, h, |x, y| {
        Rgb([
            (120 + x * 60 / w) as u8,
            (90 + y * 80 / h) as u8,
            (140 + (x + y) * 40 / (w + h)) as u8,
        ])
    });
    if let Some(payload) = payload {
        let code = QrCode::new(payload.as_bytes()).unwrap();
        let (width, module) = (code.width() as u32, 6);
        let colors = code.to_colors();
        for y in 0..(width + 8) * module {
            for x in 0..(width + 8) * module {
                let (mx, my) = (x / module, y / module);
                let inside = (4..width + 4).contains(&mx) && (4..width + 4).contains(&my);
                let dark = inside && colors[((my - 4) * width + (mx - 4)) as usize] == Color::Dark;
                let pixel = if dark {
                    Rgb([20, 20, 30])
                } else {
                    Rgb([235, 230, 220])
                };
                img.put_pixel(400 + x, 250 + y, pixel);
            }
        }
    }
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

/// A report as JSON, without the one field that depends on the clock.
fn stable(report: &Report) -> serde_json::Value {
    let mut value = serde_json::to_value(report).unwrap();
    value.as_object_mut().unwrap().remove("time_ms");
    value
}

enum Content {
    Image(Vec<u8>),
    Text(&'static str),
}

#[test]
fn a_compiled_workflow_checks_like_the_named_one() {
    let server = engine("named");
    let compiled = server
        .compile_workflow(&server.config.workflows["default"], &BTreeMap::new())
        .unwrap();

    // (case, content, expected action, expected unchecked rules)
    let cases: Vec<(&str, Content, Action, Vec<&str>)> = vec![
        (
            "an image with a QR code to a listed domain",
            Content::Image(png(Some("https://www.adult-site.com/promo"))),
            Action::Reject,
            vec!["nudity"],
        ),
        (
            "a clean image",
            Content::Image(png(None)),
            Action::Accept,
            vec!["nudity"],
        ),
        (
            "a text with an IBAN",
            Content::Text("pay to BE71 0961 2345 6769 please"),
            Action::Reject,
            vec![],
        ),
        (
            "a text with a phone number",
            Content::Text("call +32 470 12 34 56 tonight"),
            Action::Review,
            vec![],
        ),
        ("an empty text", Content::Text(""), Action::Accept, vec![]),
    ];
    let mut failures = Vec::new();
    for (case, content, action, unchecked) in cases {
        let (named, with) = match &content {
            Content::Image(bytes) => (
                server.check_image(bytes, "default"),
                server.check_image_with(bytes, "default", &compiled),
            ),
            Content::Text(text) => (
                server.check_text(text, "default"),
                server.check_text_with(text, "default", &compiled),
            ),
        };
        let (named, with) = match (named, with) {
            (Ok(named), Ok(with)) => (named, with),
            (named, with) => {
                failures.push(format!("{case}: named {named:?}, compiled {with:?}"));
                continue;
            }
        };
        if stable(&named) != stable(&with) {
            failures.push(format!(
                "{case}: the named workflow gave {}, the compiled one {}",
                stable(&named),
                stable(&with)
            ));
        }
        if named.summary.action != action || named.unchecked != unchecked {
            failures.push(format!(
                "{case}: want {action:?} and unchecked {unchecked:?}, got {:?} and {:?}",
                named.summary.action, named.unchecked
            ));
        }
    }
    // The name is the caller's: the report carries back the one it was given.
    match server.check_text_with("hello", "avatars", &compiled) {
        Ok(report) if report.workflow == "avatars" => {}
        other => failures.push(format!(
            "the report must carry the caller's name `avatars`, got {other:?}"
        )),
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Two tools whose files are not there: enough to know which categories
/// exist, and no model to load.
const CLASSIFIER: &str = r#"
[models.nsfw]
kind = "image_classifier"
path = "missing/nsfw.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }
"#;

/// The empty `concepts` table is the zero-shot model's own list, where the
/// cases put the concept they test.
const ZERO_SHOT: &str = r#"
[models.siglip]
kind = "image_zero_shot"
vision = "missing/vision.onnx"
text = "missing/text.onnx"
tokenizer = "missing/tokenizer.json"
size = 224
logit_scale = 4.7
logit_bias = -12.9
[models.siglip.concepts]
"#;

const DEFAULT_WORKFLOW: &str = "[workflows.default.text]\npii_phone = { review = 1.0 }\n";

enum Outcome {
    Valid,
    /// Refused with a text holding `fragment`. `file_prefix` is what the
    /// configuration file adds before the very same text, or None when the
    /// file accepts what a compiled workflow refuses.
    Refused {
        fragment: &'static str,
        file_prefix: Option<&'static str>,
    },
}

#[test]
fn compile_refuses_what_validate_refuses() {
    use Outcome::{Refused, Valid};
    let in_workflow = Some("workflow `probe`: ");
    let in_model = Some("model `siglip`: ");
    // (case, the workflow `probe`, the concepts of `siglip`, outcome)
    let cases: Vec<(&str, &str, &str, Outcome)> = vec![
        (
            "a valid spec",
            "[workflows.probe.image]\nnudity = { reject = 0.8, review = 0.5 }\n[workflows.probe.text]\npii_phone = { review = 1.0 }",
            "",
            Valid,
        ),
        (
            "a category no tool produces",
            "[workflows.probe.image]\nnudty = { reject = 0.8 }",
            "",
            Refused {
                fragment: "image category `nudty` is not produced by any model",
                file_prefix: in_workflow,
            },
        ),
        (
            "a category of the other kind of content",
            "[workflows.probe.image]\npii_phone = { reject = 1.0 }",
            "",
            Refused {
                fragment: "image category `pii_phone` is not produced by any model",
                file_prefix: in_workflow,
            },
        ),
        (
            "a threshold above 1",
            "[workflows.probe.image]\nnudity = { reject = 1.5 }",
            "",
            Refused {
                fragment: "threshold 1.5 for `nudity` is outside [0, 1]",
                file_prefix: in_workflow,
            },
        ),
        (
            "a threshold below 0",
            "[workflows.probe.image]\nnudity = { review = -0.25 }",
            "",
            Refused {
                fragment: "threshold -0.25 for `nudity` is outside [0, 1]",
                file_prefix: in_workflow,
            },
        ),
        (
            "review above reject",
            "[workflows.probe.image]\nnudity = { reject = 0.5, review = 0.8 }",
            "",
            Refused {
                fragment: "`nudity` has a `review` threshold (0.8) above its `reject` threshold (0.5)",
                file_prefix: in_workflow,
            },
        ),
        (
            "a rule without any threshold",
            "[workflows.probe.image]\nnudity = {}",
            "",
            Refused {
                fragment: "`nudity` has neither `reject` nor `review`",
                file_prefix: in_workflow,
            },
        ),
        (
            "an empty image and text workflow",
            "[workflows.probe]",
            "",
            Refused {
                fragment: "the workflow has no rule",
                file_prefix: None,
            },
        ),
        (
            "a concept without prompts",
            "[workflows.probe.image]\nfireworks = { review = 0.5 }",
            "fireworks = { shift = 6.0, prompts = [], negatives = [\"a photo of a city at night\"] }",
            Refused {
                fragment: "concept `fireworks` has no prompt",
                file_prefix: in_model,
            },
        ),
        (
            "a concept named like a category a tool scores",
            "[workflows.probe.image]\nnudity = { review = 0.5 }",
            "nudity = [\"a photo of a marble statue\"]",
            Refused {
                fragment: "concept `nudity` has the name of a category that already exists",
                file_prefix: None,
            },
        ),
        (
            "a concept named like a built-in category",
            "[workflows.probe.image]\nharassment = { review = 0.5 }",
            "harassment = [\"a photo of an insulting sign\"]",
            Refused {
                fragment: "concept `harassment` has the name of a category that already exists",
                file_prefix: None,
            },
        ),
    ];

    let server = Engine::load(
        Config::parse(
            &format!("{CLASSIFIER}{ZERO_SHOT}\n{DEFAULT_WORKFLOW}"),
            Path::new("none"),
        )
        .unwrap(),
        None,
    );
    let mut failures = Vec::new();
    for (case, workflow, concepts, outcome) in cases {
        let raw = format!("{CLASSIFIER}{ZERO_SHOT}{concepts}\n{DEFAULT_WORKFLOW}\n{workflow}");
        // The same text read two ways: as a configuration file, and as the
        // objects a program holds.
        let in_file = Config::parse(&raw, Path::new("none"))
            .map(|_| ())
            .map_err(|e| format!("{e:#}"));
        let parsed: Config = toml::from_str(&raw).unwrap();
        let concepts: BTreeMap<String, ConceptSpec> = match &parsed.models["siglip"].spec {
            ModelSpec::ImageZeroShot { concepts, .. } => concepts.clone(),
            other => panic!("{case}: {other:?}"),
        };
        let compiled = server.compile_workflow(&parsed.workflows["probe"], &concepts);
        match (outcome, compiled) {
            (Valid, Ok(_)) => {
                if let Err(e) = in_file {
                    failures.push(format!("{case}: compiles, but the file is refused: {e}"));
                }
            }
            (
                Refused {
                    fragment,
                    file_prefix,
                },
                Err(problems),
            ) => {
                let [problem] = problems.as_slice() else {
                    failures.push(format!("{case}: want one problem, got {problems:?}"));
                    continue;
                };
                if !problem.message.contains(fragment) {
                    failures.push(format!("{case}: want {fragment:?}, got {problem:?}"));
                }
                let same = match (file_prefix, in_file) {
                    (Some(prefix), Err(e)) => e == format!("{prefix}{}", problem.message),
                    (None, Ok(())) => true,
                    _ => false,
                };
                if !same {
                    failures.push(format!(
                        "{case}: the file and the compiler must say the same ({file_prefix:?} + {:?})",
                        problem.message
                    ));
                }
            }
            (_, compiled) => failures.push(format!("{case}: compile gave {compiled:?}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_custom_category_needs_a_zero_shot_model_that_keeps_its_text_encoder() {
    let spec: WorkflowSpec =
        serde_json::from_str(r#"{ "image": { "fireworks": { "review": 0.5 } } }"#).unwrap();
    let concepts: BTreeMap<String, ConceptSpec> =
        serde_json::from_str(r#"{ "fireworks": ["a photo of fireworks"] }"#).unwrap();
    let turned_off = ZERO_SHOT.replace(
        "kind = \"image_zero_shot\"",
        "kind = \"image_zero_shot\"\nenabled = false",
    );
    // (case, the models, what the problem says). The case of a model that is
    // ready but frees its text encoder needs the model files: see the
    // ignored test of tests/zero_shot.rs.
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "no zero-shot model in the configuration",
            CLASSIFIER.to_string(),
            "the configuration has none",
        ),
        (
            "a zero-shot model whose files are missing",
            format!("{CLASSIFIER}{ZERO_SHOT}"),
            "`siglip`, which is disabled or not ready",
        ),
        (
            "a zero-shot model that is turned off",
            format!("{CLASSIFIER}{turned_off}"),
            "`siglip`, which is disabled or not ready",
        ),
    ];
    let mut failures = Vec::new();
    for (case, models, fragment) in cases {
        let config = format!("{models}\n{DEFAULT_WORKFLOW}");
        let server = Engine::load(Config::parse(&config, Path::new("none")).unwrap(), None);
        match server.compile_workflow(&spec, &concepts) {
            Err(problems) if problems.len() == 1 && problems[0].message.contains(fragment) => {}
            other => failures.push(format!("{case}: want {fragment:?}, got {other:?}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The bytes of embeddings as a program stores them: little-endian f32, in
/// base64.
fn floats(values: &[f32]) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A compiled workflow in its stored form, with one custom category whose
/// embeddings hold three values. Its numbers are exact in binary, so that
/// reading and writing them again gives the same text.
fn stored() -> serde_json::Value {
    serde_json::json!({
        "format": 1,
        "spec": {
            "image": {
                "fireworks": { "review": 0.5 },
                "nudity": { "reject": 0.75, "review": 0.5 },
                "qr_code": { "review": 1.0 }
            },
            "text": {
                "pii_iban": { "reject": 1.0 },
                "pii_phone": { "review": 1.0 }
            }
        },
        "concepts": [{
            "name": "fireworks",
            "model": "siglip",
            "dimension": 3,
            "shift": 6.5,
            "prompts": [
                { "text": "a photo of fireworks", "embedding": floats(&[0.6, 0.8, 0.0]) },
                { "text": "a photo of pyrotechnics", "embedding": floats(&[0.0, 0.6, 0.8]) }
            ],
            "negatives": [
                { "text": "a photo of a city at night", "embedding": floats(&[-0.6, 0.0, 0.8]) }
            ]
        }]
    })
}

#[test]
fn a_compiled_workflow_survives_json() {
    let mut failures = Vec::new();
    let read = |document: serde_json::Value| -> Result<CompiledWorkflow, String> {
        serde_json::from_value(document).map_err(|e| e.to_string())
    };

    // Stored and read back: every field is kept, and it checks the same.
    match read(stored()) {
        Err(e) => failures.push(format!("a stored workflow is read: {e}")),
        Ok(original) => {
            let text = serde_json::to_string(&original).unwrap();
            match serde_json::from_str::<CompiledWorkflow>(&text) {
                Err(e) => failures.push(format!("a written workflow is read back: {e}")),
                Ok(again) => {
                    if serde_json::to_value(&again).unwrap() != stored() || again != original {
                        failures.push(format!("a round trip changed the workflow: {text}"));
                    }
                    let server = engine("json");
                    let image = png(Some("https://night.events/e/12"));
                    // (case, content, expected action)
                    let cases: Vec<(&str, Content, Action)> = vec![
                        (
                            "an image with a QR code",
                            Content::Image(image),
                            Action::Review,
                        ),
                        (
                            "a text with a phone number",
                            Content::Text("call +32 470 12 34 56 tonight"),
                            Action::Review,
                        ),
                    ];
                    for (case, content, action) in cases {
                        let (a, b) = match &content {
                            Content::Image(bytes) => (
                                server.check_image_with(bytes, "stored", &original),
                                server.check_image_with(bytes, "stored", &again),
                            ),
                            Content::Text(text) => (
                                server.check_text_with(text, "stored", &original),
                                server.check_text_with(text, "stored", &again),
                            ),
                        };
                        match (a, b) {
                            (Ok(a), Ok(b))
                                if stable(&a) == stable(&b) && a.summary.action == action => {}
                            (a, b) => failures.push(format!(
                                "{case}: want the same report with {action:?}, got {a:?} and {b:?}"
                            )),
                        }
                    }
                }
            }
        }
    }

    // (case, what to change in a stored workflow, what the refusal says)
    let cases: Vec<(&str, fn(&mut serde_json::Value), &str)> = vec![
        (
            "another format",
            |d| d["format"] = serde_json::json!(2),
            "format 2",
        ),
        (
            "no format",
            |d| {
                d.as_object_mut().unwrap().remove("format");
            },
            "`format`",
        ),
        (
            "an embedding shorter than the declared dimension",
            |d| {
                d["concepts"][0]["prompts"][0]["embedding"] = serde_json::json!(floats(&[0.6, 0.8]))
            },
            "holds 2 values, the concept declares 3",
        ),
        (
            "an embedding cut in the middle of a value",
            |d| {
                let mut bytes = floats(&[0.6, 0.8, 0.0]);
                bytes.truncate(bytes.len() - 4);
                d["concepts"][0]["negatives"][0]["embedding"] = serde_json::json!(bytes);
            },
            "whole number of 32-bit floats",
        ),
        (
            "an embedding that is not base64",
            |d| d["concepts"][0]["prompts"][1]["embedding"] = serde_json::json!("0.6, 0.8, 0.0"),
            "not valid base64",
        ),
        (
            "a NaN",
            |d| {
                d["concepts"][0]["prompts"][0]["embedding"] =
                    serde_json::json!(floats(&[0.6, f32::NAN, 0.0]))
            },
            "NaN or an infinite value",
        ),
        (
            "an infinite value",
            |d| {
                d["concepts"][0]["negatives"][0]["embedding"] =
                    serde_json::json!(floats(&[0.6, f32::INFINITY, 0.0]))
            },
            "NaN or an infinite value",
        ),
        (
            "a concept without prompt",
            |d| d["concepts"][0]["prompts"] = serde_json::json!([]),
            "has no prompt",
        ),
        (
            "embeddings of no value",
            |d| d["concepts"][0]["dimension"] = serde_json::json!(0),
            "0 values",
        ),
        (
            "the same concept twice",
            |d| {
                let twice = d["concepts"][0].clone();
                d["concepts"].as_array_mut().unwrap().push(twice);
            },
            "`fireworks` appears twice",
        ),
        (
            "a field this format does not have",
            |d| d["concepts"][0]["weight"] = serde_json::json!(2),
            "unknown field `weight`",
        ),
    ];
    for (case, change, fragment) in cases {
        let mut document = stored();
        change(&mut document);
        match read(document) {
            Err(e) if e.contains(fragment) => {}
            other => failures.push(format!(
                "{case}: want a refusal with {fragment:?}, got {other:?}"
            )),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_compiled_workflow_is_unavailable_when_no_rule_can_be_scored_here() {
    let server = engine("unavailable");
    let only = |spec: serde_json::Value| {
        let mut document = stored();
        document["spec"] = spec;
        serde_json::from_value::<CompiledWorkflow>(document).unwrap()
    };
    // (case, the rules of the workflow, text rather than image, what the refusal says)
    let cases: Vec<(&str, serde_json::Value, bool, &str)> = vec![
        (
            "only a custom category, and the model that scores it is not here",
            serde_json::json!({ "image": { "fireworks": { "review": 0.5 } } }),
            false,
            "`fireworks` needs siglip",
        ),
        (
            "a custom category and a tool that is off",
            serde_json::json!({ "image": { "fireworks": { "review": 0.5 }, "nudity": { "review": 0.5 } } }),
            false,
            "`nudity` needs nsfw",
        ),
        (
            "no rule for this kind of content",
            serde_json::json!({ "image": { "qr_code": { "review": 1.0 } } }),
            true,
            "workflow `avatars` has no text rules",
        ),
    ];
    let mut failures = Vec::new();
    for (case, spec, text, fragment) in cases {
        let workflow = only(spec);
        let result = if text {
            server.check_text_with("hello", "avatars", &workflow)
        } else {
            server.check_image_with(&png(None), "avatars", &workflow)
        };
        match result {
            Err(CheckError::Unavailable(message)) if message.contains(fragment) => {}
            other => failures.push(format!("{case}: want {fragment:?}, got {other:?}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn the_shipped_templates_compile() {
    // Every tool of the shipped configuration counts, whether or not its
    // files are there: here none is, and no model is loaded.
    let server = Engine::load(
        Config::parse(DEFAULT_CONFIG, Path::new("none")).unwrap(),
        None,
    );
    let templates = catalog::shipped_workflows();
    let mut failures = Vec::new();
    if !templates.contains_key("default") {
        failures.push(format!("no `default` among {:?}", templates.keys()));
    }
    for (name, spec) in &templates {
        if let Err(problems) = server.compile_workflow(spec, &BTreeMap::new()) {
            failures.push(format!("`{name}` does not compile: {problems:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Tools of every kind, in a configuration with a label of its own.
const CATALOGUE: &str = r#"
[models.nsfw]
kind = "image_classifier"
path = "a.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }

[models.owen]
kind = "image_classifier"
path = "b.onnx"
size = 224
classes = ["NSFL", "NSFW", "SFW"]
categories = { NSFL = "gore", NSFW = "nudity" }

[models.siglip]
kind = "image_zero_shot"
vision = "v.onnx"
text = "t.onnx"
tokenizer = "tok.json"
size = 224
logit_scale = 4.7
logit_bias = -12.9
[models.siglip.concepts]
gore = ["a photo of gore"]
weapon = ["a photo of a gun"]

[models.qr]
kind = "qr_reader"

[models.adult_sites]
kind = "domain_list"
paths = ["adult.txt"]
public_suffixes = "psl.dat"
category = "link_adult"

[labels.weapon]
text = "Firearm"
message = "This content shows a firearm."

[workflows.default.image]
nudity = { reject = 0.8 }
"#;

#[test]
fn the_catalogue_names_each_category_and_the_tools_that_score_it() {
    let config = Config::parse(CATALOGUE, Path::new("none")).unwrap();
    let catalogue = catalog::categories(&config);
    // (case, kind, id, title, tools, scored by openmod's own text rules)
    let cases: Vec<(&str, Kind, &str, &str, Vec<&str>, bool)> = vec![
        (
            "two classifiers",
            Kind::Image,
            "nudity",
            "Nudity",
            vec!["nsfw", "owen"],
            false,
        ),
        (
            "a classifier and a zero-shot concept",
            Kind::Image,
            "gore",
            "Gore",
            vec!["owen", "siglip"],
            false,
        ),
        (
            "a title of the configuration's own",
            Kind::Image,
            "weapon",
            "Firearm",
            vec!["siglip"],
            false,
        ),
        (
            "read out of a QR code",
            Kind::Image,
            "qr_link_adult",
            "Adult site link in a QR code",
            vec!["qr+adult_sites"],
            false,
        ),
        (
            "a domain list, for texts",
            Kind::Text,
            "link_adult",
            "Adult site link",
            vec!["adult_sites"],
            false,
        ),
        (
            "a rule that needs no tool",
            Kind::Text,
            "pii_phone",
            "Phone number",
            vec![],
            true,
        ),
    ];
    let mut failures = Vec::new();
    for (case, kind, id, title, tools, rule) in cases {
        match catalogue.iter().find(|c| c.kind == kind && c.id == id) {
            Some(c) if c.title == title && c.tools == tools && c.rule == rule => {
                if c.message.is_empty() || c.message.starts_with("This content was flagged for") {
                    failures.push(format!("{case}: no message of its own: {c:?}"));
                }
            }
            other => failures.push(format!(
                "{case}: want {title:?} by {tools:?}, got {other:?}"
            )),
        }
    }
    let mut listed = std::collections::BTreeSet::new();
    for c in &catalogue {
        if !listed.insert((c.id.as_str(), c.kind == Kind::Image)) {
            failures.push(format!("`{}` is listed twice for the same kind", c.id));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
