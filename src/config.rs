//! The `openmod.toml` file: server limits, models and workflows.

use crate::workflow::Workflow;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The configuration `openmod serve` writes on first run, and the source of
/// the model blocks the admin interface adds back when a tool is enabled.
pub const DEFAULT_CONFIG: &str = include_str!("../openmod.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub admin: Admin,
    #[serde(default)]
    pub models: BTreeMap<String, ModelEntry>,
    #[serde(default)]
    pub workflows: BTreeMap<String, WorkflowSpec>,
    /// Optional overrides of the built-in titles and messages.
    #[serde(default)]
    pub labels: BTreeMap<String, LabelSpec>,
    /// How the scores of several models for one category combine. A
    /// category not listed takes the highest score.
    #[serde(default)]
    pub combine: BTreeMap<String, Combine>,
}

/// How the scores several models give one category become one score.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    /// The highest score: any model can flag the content on its own.
    #[default]
    Max,
    /// The mean of each model's score: the models must agree, which removes
    /// the confident mistakes each one makes alone.
    Mean,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Server {
    pub bind: String,
    /// Largest request body accepted, in bytes.
    pub max_upload_bytes: usize,
    /// Largest decoded image accepted, in pixels (width x height).
    pub max_image_pixels: u64,
    /// Longest text accepted, in characters.
    pub max_text_chars: usize,
    /// How many checks may run inference at the same time.
    pub max_concurrent: usize,
    /// CPU threads each ONNX model may use.
    pub threads_per_model: usize,
    /// File holding one API key per line. `OPENMOD_API_KEYS` (comma
    /// separated) is read as well.
    pub api_keys_file: Option<PathBuf>,
    /// Serve without any API key. Refused on a non-loopback address.
    pub allow_unauthenticated: bool,
    /// Where openmod keeps the admin token and the API keys created in the
    /// admin interface, as SHA-256 digests only.
    pub state_file: PathBuf,
    /// How often the domain lists are downloaded again, when they changed
    /// upstream. 0 never refreshes them.
    pub lists_refresh_hours: u64,
}

impl Default for Server {
    fn default() -> Self {
        Server {
            bind: "127.0.0.1:8088".into(),
            max_upload_bytes: 15 * 1024 * 1024,
            max_image_pixels: 50_000_000,
            max_text_chars: 20_000,
            max_concurrent: 2,
            threads_per_model: 2,
            api_keys_file: None,
            allow_unauthenticated: false,
            state_file: PathBuf::from("openmod-state.toml"),
            lists_refresh_hours: 24,
        }
    }
}

/// The admin web interface, on its own listener.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Admin {
    pub enabled: bool,
    /// Loopback by default: reach it through an SSH tunnel or an HTTPS
    /// reverse proxy rather than exposing it.
    pub bind: String,
}

impl Default for Admin {
    fn default() -> Self {
        Admin {
            enabled: true,
            bind: "127.0.0.1:8089".into(),
        }
    }
}

/// One `[models.<name>]` block.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelEntry {
    /// A disabled model is not loaded and uses no memory. The workflow rules
    /// that need it are reported as `unchecked` instead of failing.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(flatten)]
    pub spec: ModelSpec,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Resize {
    /// Resize to `size x size`, ignoring the aspect ratio.
    #[default]
    Stretch,
    /// Resize the shorter side to `size`, then crop the center.
    ShorterSideCrop,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Bilinear,
    #[default]
    Bicubic,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PixelRange {
    /// Pixels in `[0, 1]`, then `(x - mean) / std`.
    #[default]
    Unit,
    /// Raw pixels in `[0, 255]`; the model normalizes them itself.
    Byte,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ImageInput {
    pub size: u32,
    #[serde(default)]
    pub resize: Resize,
    #[serde(default)]
    pub filter: Filter,
    #[serde(default)]
    pub pixel_range: PixelRange,
    #[serde(default = "half")]
    pub mean: [f32; 3],
    #[serde(default = "half")]
    pub std: [f32; 3],
}

fn half() -> [f32; 3] {
    [0.5, 0.5, 0.5]
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
// serde cannot deny unknown fields next to `flatten`: `Config::parse` checks
// the model settings against `MODEL_KEYS` instead.
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelSpec {
    /// An ONNX image classifier with one output of class scores.
    ImageClassifier {
        path: PathBuf,
        #[serde(flatten)]
        input: ImageInput,
        /// Class names in output order.
        classes: Vec<String>,
        /// Which classes feed which categories, e.g. `{ NSFW = "nudity" }`.
        categories: BTreeMap<String, String>,
        /// Apply a softmax to the output (when the model returns logits).
        #[serde(default)]
        softmax: bool,
    },
    /// SigLIP zero-shot: one category per list of English prompts.
    ImageZeroShot {
        vision: PathBuf,
        text: PathBuf,
        tokenizer: PathBuf,
        #[serde(flatten)]
        input: ImageInput,
        logit_scale: f32,
        logit_bias: f32,
        concepts: BTreeMap<String, ConceptSpec>,
    },
    /// Qwen3Guard-Gen exported to ONNX by `tools/export_qwen3guard.py`.
    TextGuard {
        path: PathBuf,
        tokenizer: PathBuf,
        #[serde(default)]
        second_opinion: Option<SecondOpinion>,
    },
    /// A GLiNER2 schema classifier (GLiGuard) exported to ONNX by
    /// `tools/export_gliguard.py`. Tasks and labels are free text: the model
    /// reads them, so they can be changed without retraining.
    TextClassifier {
        path: PathBuf,
        tokenizer: PathBuf,
        tasks: Vec<GlinerTask>,
        #[serde(default)]
        second_opinion: Option<SecondOpinion>,
    },
    /// Domain lists, one domain per line: a text that links to a listed site
    /// scores `category`.
    DomainList {
        paths: Vec<PathBuf>,
        /// The Public Suffix List, so that a listed public suffix never
        /// covers the sites registered under it.
        public_suffixes: PathBuf,
        category: String,
        /// Domains never flagged, with their subdomains, whatever the
        /// lists say.
        #[serde(default)]
        allow: Vec<String>,
    },
    /// Reads the QR codes of an image. What they hold goes through the text
    /// rules and the domain lists, and each category comes back with a
    /// `qr_` prefix (`qr_link_adult`); `qr_code` is any code at all.
    QrReader {},
    /// PP-OCR: reads the text of an image. It goes through everything a text
    /// goes through, and each category comes back with a `text_` prefix
    /// (`text_link_adult`, `text_extremism`; `text_unsafe` stays as is).
    ImageOcr {
        detector: PathBuf,
        recognizer: PathBuf,
        dictionary: PathBuf,
        /// The longer side the detector sees, in pixels.
        #[serde(default = "ocr_max_side")]
        max_side: u32,
        /// The confidence a line needs to be kept.
        #[serde(default = "ocr_min_score")]
        min_score: f32,
    },
}

fn ocr_max_side() -> u32 {
    960
}

fn ocr_min_score() -> f32 {
    0.5
}

/// What reads text out of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Qr,
    Ocr,
}

/// What a QR code holding anything at all scores.
pub const QR_CODE: &str = "qr_code";

/// The image category a reader gives a text category: `qr_link_adult` from
/// a QR code, `text_link_adult` from the text of an image. A text category
/// that already starts with `text_` keeps its name.
pub fn read_category(reader: Reader, category: &str) -> String {
    match reader {
        Reader::Qr => format!("qr_{category}"),
        Reader::Ocr if category.starts_with("text_") => category.to_string(),
        Reader::Ocr => format!("text_{category}"),
    }
}

/// The settings each model kind accepts, besides `kind` and `enabled`.
const MODEL_KEYS: &[(&str, &[&str])] = &[
    (
        "image_classifier",
        &[
            "path",
            "size",
            "resize",
            "filter",
            "pixel_range",
            "mean",
            "std",
            "classes",
            "categories",
            "softmax",
        ],
    ),
    (
        "image_zero_shot",
        &[
            "vision",
            "text",
            "tokenizer",
            "size",
            "resize",
            "filter",
            "pixel_range",
            "mean",
            "std",
            "logit_scale",
            "logit_bias",
            "concepts",
        ],
    ),
    ("text_guard", &["path", "tokenizer", "second_opinion"]),
    (
        "text_classifier",
        &["path", "tokenizer", "tasks", "second_opinion"],
    ),
    (
        "domain_list",
        &["paths", "public_suffixes", "category", "allow"],
    ),
    ("qr_reader", &[]),
    (
        "image_ocr",
        &[
            "detector",
            "recognizer",
            "dictionary",
            "max_side",
            "min_score",
        ],
    ),
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlinerTask {
    pub name: String,
    pub labels: Vec<String>,
    /// A sigmoid per label (several labels may apply) instead of a softmax.
    #[serde(default)]
    pub multi_label: bool,
    /// Which labels feed which categories, e.g. `{ unsafe = "text_unsafe" }`.
    #[serde(default)]
    pub categories: BTreeMap<String, String>,
}

/// Run a model only as a second opinion: when the models without this
/// setting left `category` with a score in `[low, high]` (both inclusive, so
/// `low = 0, high = 1` always runs it). Its scores are then averaged with
/// theirs for the categories both produce.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondOpinion {
    pub category: String,
    pub low: f32,
    pub high: f32,
}

/// A zero-shot category: its prompts, a logit shift that moves the decision
/// boundary measured on a corpus to a score of 0.5, and negatives: what your
/// content often shows that looks like the category without being it
/// (people dancing for `violence`). An image closer to a negative than to
/// every prompt scores low.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ConceptSpec {
    Prompts(Vec<String>),
    Calibrated(CalibratedConcept),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibratedConcept {
    pub prompts: Vec<String>,
    #[serde(default)]
    pub shift: f32,
    #[serde(default)]
    pub negatives: Vec<String>,
}

impl ConceptSpec {
    pub fn prompts(&self) -> &[String] {
        match self {
            ConceptSpec::Prompts(p) => p,
            ConceptSpec::Calibrated(c) => &c.prompts,
        }
    }

    pub fn shift(&self) -> f32 {
        match self {
            ConceptSpec::Prompts(_) => 0.0,
            ConceptSpec::Calibrated(c) => c.shift,
        }
    }

    pub fn negatives(&self) -> &[String] {
        match self {
            ConceptSpec::Prompts(_) => &[],
            ConceptSpec::Calibrated(c) => &c.negatives,
        }
    }
}

/// The categories Qwen3Guard can return, mapped to openmod ids.
pub const TEXT_GUARD_CATEGORIES: &[(&str, &str)] = &[
    ("Violent", "violence"),
    ("Non-violent Illegal Acts", "illegal"),
    ("Sexual Content or Sexual Acts", "sexual"),
    ("PII", "pii"),
    ("Suicide & Self-Harm", "self_harm"),
    ("Unethical Acts", "unethical"),
    ("Politically Sensitive Topics", "political"),
    ("Copyright Violation", "copyright"),
    ("Jailbreak", "jailbreak"),
];

/// The categories the built-in text rules can return.
pub const RULE_CATEGORIES: &[&str] = &[
    "pii_email",
    "pii_phone",
    "pii_iban",
    "pii_card",
    "link",
    "messaging_handoff",
    "spam",
    "scam",
    "profanity",
    "extremism",
];

impl ModelSpec {
    /// Whether the tool checks images: an image model or a reader.
    pub fn is_image(&self) -> bool {
        matches!(
            self,
            ModelSpec::ImageClassifier { .. }
                | ModelSpec::ImageZeroShot { .. }
                | ModelSpec::QrReader {}
                | ModelSpec::ImageOcr { .. }
        )
    }

    /// Whether the tool is a text model (a domain list is not).
    pub fn is_text_model(&self) -> bool {
        matches!(
            self,
            ModelSpec::TextGuard { .. } | ModelSpec::TextClassifier { .. }
        )
    }

    /// Whether the tool reads text out of images, and how.
    pub fn reader(&self) -> Option<Reader> {
        match self {
            ModelSpec::QrReader {} => Some(Reader::Qr),
            ModelSpec::ImageOcr { .. } => Some(Reader::Ocr),
            _ => None,
        }
    }

    /// Whether the tool runs on ONNX Runtime.
    pub fn uses_onnx(&self) -> bool {
        !matches!(self, ModelSpec::DomainList { .. } | ModelSpec::QrReader {})
    }

    /// The files the model reads.
    pub fn files(&self) -> Vec<&Path> {
        match self {
            ModelSpec::ImageClassifier { path, .. } => vec![path],
            ModelSpec::ImageZeroShot {
                vision,
                text,
                tokenizer,
                ..
            } => vec![vision, text, tokenizer],
            ModelSpec::TextGuard {
                path, tokenizer, ..
            }
            | ModelSpec::TextClassifier {
                path, tokenizer, ..
            } => {
                vec![path, tokenizer]
            }
            ModelSpec::DomainList {
                paths,
                public_suffixes,
                ..
            } => paths
                .iter()
                .map(PathBuf::as_path)
                .chain([public_suffixes.as_path()])
                .collect(),
            ModelSpec::QrReader {} => Vec::new(),
            ModelSpec::ImageOcr {
                detector,
                recognizer,
                dictionary,
                ..
            } => vec![detector, recognizer, dictionary],
        }
    }

    pub fn second_opinion(&self) -> Option<&SecondOpinion> {
        match self {
            ModelSpec::TextGuard { second_opinion, .. }
            | ModelSpec::TextClassifier { second_opinion, .. } => second_opinion.as_ref(),
            _ => None,
        }
    }

    /// Every category this tool scores by itself. A reader also derives
    /// categories from the text it reads: see `Config::derived`.
    pub fn categories(&self) -> BTreeSet<String> {
        match self {
            ModelSpec::ImageClassifier { categories, .. } => categories.values().cloned().collect(),
            ModelSpec::ImageZeroShot { concepts, .. } => concepts.keys().cloned().collect(),
            ModelSpec::TextGuard { .. } => TEXT_GUARD_CATEGORIES
                .iter()
                .map(|(_, id)| id.to_string())
                .chain(["text_unsafe".to_string()])
                .collect(),
            ModelSpec::TextClassifier { tasks, .. } => tasks
                .iter()
                .flat_map(|t| t.categories.values().cloned())
                .collect(),
            ModelSpec::DomainList { category, .. } => BTreeSet::from([category.clone()]),
            ModelSpec::QrReader {} => BTreeSet::from([QR_CODE.to_string()]),
            ModelSpec::ImageOcr { .. } => BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    #[serde(default)]
    pub image: Workflow,
    #[serde(default)]
    pub text: Workflow,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelSpec {
    pub text: String,
    pub message: String,
}

impl Config {
    /// Reads `path`, then applies the `OPENMOD_BIND` and `OPENMOD_ADMIN_BIND`
    /// overrides (handy in a container).
    pub fn load(path: &Path) -> Result<Config> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        let mut config =
            Config::parse(&raw, base).with_context(|| format!("in {}", path.display()))?;
        config.apply_env();
        Ok(config)
    }

    /// Parses and validates a configuration whose relative paths are relative
    /// to `base`.
    pub fn parse(raw: &str, base: &Path) -> Result<Config> {
        let table: toml::Table = toml::from_str(raw)?;
        check_model_keys(&table)?;
        let mut config: Config = toml::from_str(raw)?;
        config.resolve_paths(base);
        config.validate()?;
        Ok(config)
    }

    pub fn apply_env(&mut self) {
        if let Ok(bind) = std::env::var("OPENMOD_BIND") {
            self.server.bind = bind;
        }
        if let Ok(bind) = std::env::var("OPENMOD_ADMIN_BIND") {
            self.admin.bind = bind;
        }
    }

    /// Model paths are relative to the config file.
    fn resolve_paths(&mut self, base: &Path) {
        let fix = |p: &mut PathBuf| {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        };
        for entry in self.models.values_mut() {
            match &mut entry.spec {
                ModelSpec::ImageClassifier { path, .. } => fix(path),
                ModelSpec::ImageZeroShot {
                    vision,
                    text,
                    tokenizer,
                    ..
                } => {
                    fix(vision);
                    fix(text);
                    fix(tokenizer);
                }
                ModelSpec::TextGuard {
                    path, tokenizer, ..
                }
                | ModelSpec::TextClassifier {
                    path, tokenizer, ..
                } => {
                    fix(path);
                    fix(tokenizer);
                }
                ModelSpec::DomainList {
                    paths,
                    public_suffixes,
                    ..
                } => {
                    paths.iter_mut().for_each(fix);
                    fix(public_suffixes);
                }
                ModelSpec::QrReader {} => {}
                ModelSpec::ImageOcr {
                    detector,
                    recognizer,
                    dictionary,
                    ..
                } => {
                    fix(detector);
                    fix(recognizer);
                    fix(dictionary);
                }
            }
        }
        if let Some(p) = self.server.api_keys_file.as_mut() {
            fix(p);
        }
        fix(&mut self.server.state_file);
    }

    /// Every category some tool of this kind can score, enabled or not,
    /// plus the rules for text and what the readers derive for images.
    pub fn known_categories(&self, image: bool) -> BTreeSet<String> {
        self.categories_where(image, &|_, _| true)
    }

    /// The categories the enabled tools (and the text rules) can score.
    pub fn available_categories(&self, image: bool) -> BTreeSet<String> {
        self.categories_where(image, &|_, e| e.enabled)
    }

    /// The categories the tools `keep` accepts can score.
    pub fn categories_where(
        &self,
        image: bool,
        keep: &dyn Fn(&str, &ModelEntry) -> bool,
    ) -> BTreeSet<String> {
        if !image {
            return self.text_categories(keep, true);
        }
        let mut out: BTreeSet<String> = self
            .models
            .iter()
            .filter(|(name, e)| e.spec.is_image() && keep(name, e))
            .flat_map(|(_, e)| e.spec.categories())
            .collect();
        out.extend(self.derived(keep).into_keys());
        out
    }

    /// The text categories: the rules', plus those of the kept domain lists
    /// and, with `models`, of the kept text models.
    fn text_categories(
        &self,
        keep: &dyn Fn(&str, &ModelEntry) -> bool,
        models: bool,
    ) -> BTreeSet<String> {
        let tools = self
            .models
            .iter()
            .filter(|(name, e)| {
                keep(name, e)
                    && (matches!(e.spec, ModelSpec::DomainList { .. })
                        || (models && e.spec.is_text_model()))
            })
            .flat_map(|(_, e)| e.spec.categories());
        RULE_CATEGORIES
            .iter()
            .map(|s| s.to_string())
            .chain(tools)
            .collect()
    }

    /// Every image category a kept reader derives from a text category, with
    /// the reader and the text category. A QR code goes through the rules and
    /// the domain lists, the text of an image through the text models too.
    pub fn derived(
        &self,
        keep: &dyn Fn(&str, &ModelEntry) -> bool,
    ) -> BTreeMap<String, (String, String)> {
        let mut out = BTreeMap::new();
        for (name, e) in &self.models {
            let Some(reader) = e.spec.reader().filter(|_| keep(name, e)) else {
                continue;
            };
            for category in self.text_categories(keep, reader == Reader::Ocr) {
                out.insert(read_category(reader, &category), (name.clone(), category));
            }
        }
        out
    }

    /// The tools that can score `category`, enabled or not. A category read
    /// out of an image names its reader and the tool behind it
    /// (`ocr+adult_sites`), or the reader alone for a rule.
    pub fn producers(&self, category: &str, image: bool) -> Vec<String> {
        let mut out: Vec<String> = self
            .models
            .iter()
            .filter(|(_, e)| e.spec.is_image() == image && e.spec.categories().contains(category))
            .map(|(name, _)| name.clone())
            .collect();
        if image && let Some((reader, text)) = self.derived(&|_, _| true).remove(category) {
            let behind = self.producers(&text, false);
            if behind.is_empty() {
                out.push(reader);
            } else {
                out.extend(behind.into_iter().map(|p| format!("{reader}+{p}")));
            }
        }
        out
    }

    /// How the `ready` text models run: each with the second opinion it
    /// gives, if any. A model configured as a second opinion whose watched
    /// category no ready first-pass model produces runs on every text
    /// instead, so turning GLiGuard off leaves Qwen3Guard doing the work.
    pub fn text_plan(&self, ready: &BTreeSet<String>) -> Vec<(String, Option<SecondOpinion>)> {
        let text: Vec<(&String, &ModelSpec)> = self
            .models
            .iter()
            .filter(|(name, e)| e.spec.is_text_model() && ready.contains(*name))
            .map(|(name, e)| (name, &e.spec))
            .collect();
        let first_pass: BTreeSet<String> = text
            .iter()
            .filter(|(_, spec)| spec.second_opinion().is_none())
            .flat_map(|(_, spec)| spec.categories())
            .collect();
        text.into_iter()
            .map(|(name, spec)| {
                let so = spec
                    .second_opinion()
                    .filter(|so| first_pass.contains(&so.category))
                    .cloned();
                (name.clone(), so)
            })
            .collect()
    }

    /// Refuses a workflow that names a category no model can score, or a
    /// threshold outside `[0, 1]`, so a typo never silently disables a rule.
    /// A category whose models are all disabled stays valid: its rules are
    /// reported as unchecked until one of them is enabled again.
    pub fn validate(&self) -> Result<()> {
        let image = self.known_categories(true);
        let text = self.known_categories(false);
        for (name, wf) in &self.workflows {
            for (kind, rules, known) in [("image", &wf.image, &image), ("text", &wf.text, &text)] {
                for (category, t) in &rules.rules {
                    if !known.contains(category) {
                        bail!(
                            "workflow `{name}`: {kind} category `{category}` is not produced by any model (known: {})",
                            known.iter().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                    for v in [t.reject, t.review].into_iter().flatten() {
                        if !(0.0..=1.0).contains(&v) {
                            bail!(
                                "workflow `{name}`: threshold {v} for `{category}` is outside [0, 1]"
                            );
                        }
                    }
                    if t.reject.is_none() && t.review.is_none() {
                        bail!("workflow `{name}`: `{category}` has neither `reject` nor `review`");
                    }
                }
            }
        }
        if !self.workflows.contains_key("default") {
            bail!("a workflow named `default` is required");
        }
        for category in self.combine.keys() {
            if !image.contains(category) && !text.contains(category) {
                bail!("[combine]: category `{category}` is not produced by any model");
            }
        }
        for (name, entry) in &self.models {
            let spec = &entry.spec;
            if let ModelSpec::ImageClassifier {
                classes,
                categories,
                ..
            } = spec
            {
                for class in categories.keys() {
                    if !classes.contains(class) {
                        bail!(
                            "model `{name}`: class `{class}` is mapped to a category but missing from `classes`"
                        );
                    }
                }
            }
            if let ModelSpec::ImageZeroShot { concepts, .. } = spec {
                for (concept, c) in concepts {
                    if c.prompts().is_empty() {
                        bail!(
                            "model `{name}`: concept `{concept}` has no prompt (negatives alone score nothing)"
                        );
                    }
                }
            }
            if let ModelSpec::TextClassifier { tasks, .. } = spec {
                for task in tasks {
                    for label in task.categories.keys() {
                        if !task.labels.contains(label) {
                            bail!(
                                "model `{name}`: label `{label}` is mapped to a category but missing from task `{}`",
                                task.name
                            );
                        }
                    }
                }
            }
            if let ModelSpec::DomainList {
                paths,
                category,
                allow,
                ..
            } = spec
            {
                if paths.is_empty() {
                    bail!("model `{name}`: a domain list needs at least one file in `paths`");
                }
                if category.is_empty() {
                    bail!("model `{name}`: a domain list needs a `category`");
                }
                if let Some(bad) = allow.iter().find(|d| !crate::domains::is_domain(d)) {
                    bail!("model `{name}`: `allow`: `{bad}` is not a domain name");
                }
            }
            if let ModelSpec::ImageOcr {
                max_side,
                min_score,
                ..
            } = spec
            {
                if !(32..=4096).contains(max_side) {
                    bail!("model `{name}`: `max_side` must be between 32 and 4096");
                }
                if !(0.0..=1.0).contains(min_score) {
                    bail!("model `{name}`: `min_score` must be in [0, 1]");
                }
            }
            if let Some(so) = spec.second_opinion() {
                let first: BTreeSet<String> = self
                    .models
                    .values()
                    .filter(|e| e.spec.is_text_model() && e.spec.second_opinion().is_none())
                    .flat_map(|e| e.spec.categories())
                    .collect();
                if !first.contains(&so.category) {
                    bail!(
                        "model `{name}`: `second_opinion` watches `{}`, which no first-pass text model produces",
                        so.category
                    );
                }
                if !(0.0..=1.0).contains(&so.low)
                    || !(0.0..=1.0).contains(&so.high)
                    || so.low >= so.high
                {
                    bail!("model `{name}`: `second_opinion` needs 0 <= low < high <= 1");
                }
            }
        }
        Ok(())
    }

    /// Rules no enabled model can score, as `workflow.kind.category`.
    pub fn inactive_rules(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (kind, image) in [("image", true), ("text", false)] {
            let available = self.available_categories(image);
            for (name, wf) in &self.workflows {
                let rules = if image { &wf.image } else { &wf.text };
                for category in rules.rules.keys().filter(|c| !available.contains(*c)) {
                    out.push(format!("{name}.{kind}.{category}"));
                }
            }
        }
        out
    }
}

/// serde ignores unknown keys next to a flattened field, so a misspelled
/// `enabeld = false` would leave a model on. Check them by hand.
fn check_model_keys(table: &toml::Table) -> Result<()> {
    let Some(models) = table.get("models").and_then(|m| m.as_table()) else {
        return Ok(());
    };
    for (name, model) in models {
        let Some(model) = model.as_table() else {
            continue;
        };
        let Some(kind) = model.get("kind").and_then(|k| k.as_str()) else {
            continue;
        };
        let Some((_, allowed)) = MODEL_KEYS.iter().find(|(k, _)| *k == kind) else {
            continue;
        };
        for key in model.keys() {
            if key != "kind" && key != "enabled" && !allowed.contains(&key.as_str()) {
                bail!(
                    "model `{name}`: unknown setting `{key}` (a `{kind}` takes kind, enabled, {})",
                    allowed.join(", ")
                );
            }
        }
        // An untagged concept with a misspelled key would fail with a vague
        // message, or not at all: name the key.
        let concepts = model.get("concepts").and_then(|c| c.as_table());
        for (concept, spec) in concepts.into_iter().flatten() {
            for key in spec.as_table().into_iter().flat_map(|t| t.keys()) {
                if !CONCEPT_KEYS.contains(&key.as_str()) {
                    bail!(
                        "model `{name}`, concept `{concept}`: unknown setting `{key}` (a concept takes {})",
                        CONCEPT_KEYS.join(", ")
                    );
                }
            }
        }
    }
    Ok(())
}

const CONCEPT_KEYS: &[&str] = &["prompts", "shift", "negatives"];
