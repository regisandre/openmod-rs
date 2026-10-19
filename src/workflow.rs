//! Workflows: which categories to check, and the thresholds that decide.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<f32>,
}

/// The rules for one kind of content, images or texts: a category, and the
/// thresholds at which it rejects or sends to review.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Workflow {
    #[serde(flatten)]
    pub rules: BTreeMap<String, Threshold>,
}

/// What a workflow checks: its rules for images and its rules for texts.
/// This is a `[workflows.<name>]` block of `openmod.toml`, and the same
/// object as JSON.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    #[serde(default)]
    pub image: Workflow,
    #[serde(default)]
    pub text: Workflow,
}

/// Something wrong with a workflow or a custom category, in openmod's own
/// validation words. The same mistake gives the same text whether it is found
/// in `openmod.toml` or in a workflow compiled at run time; the configuration
/// file only adds a prefix that says which workflow or model it was in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub message: String,
}

impl Problem {
    pub fn new(message: impl Into<String>) -> Problem {
        Problem {
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Problem {}

impl WorkflowSpec {
    /// Whether the workflow has no rule at all, for images or for texts.
    pub fn is_empty(&self) -> bool {
        self.image.rules.is_empty() && self.text.rules.is_empty()
    }

    /// What is wrong with the rules, given the categories that exist for
    /// images and for texts: a category no tool produces, a threshold outside
    /// [0, 1], a rule with no threshold, a `review` above its `reject` (which
    /// then never sends anything to review). This one function checks the
    /// workflows of `openmod.toml` and those compiled at run time.
    pub fn problems(&self, image: &BTreeSet<String>, text: &BTreeSet<String>) -> Vec<Problem> {
        let mut out = Vec::new();
        for (kind, rules, known) in [("image", &self.image, image), ("text", &self.text, text)] {
            for (category, t) in &rules.rules {
                if !known.contains(category) {
                    out.push(Problem::new(format!(
                        "{kind} category `{category}` is not produced by any model (known: {})",
                        known.iter().cloned().collect::<Vec<_>>().join(", ")
                    )));
                }
                for v in [t.reject, t.review].into_iter().flatten() {
                    if !(0.0..=1.0).contains(&v) {
                        out.push(Problem::new(format!(
                            "threshold {v} for `{category}` is outside [0, 1]"
                        )));
                    }
                }
                if t.reject.is_none() && t.review.is_none() {
                    out.push(Problem::new(format!(
                        "`{category}` has neither `reject` nor `review`"
                    )));
                }
                if let (Some(reject), Some(review)) = (t.reject, t.review)
                    && review > reject
                {
                    out.push(Problem::new(format!(
                        "`{category}` has a `review` threshold ({review}) above its `reject` threshold ({reject}): nothing would ever be sent to review"
                    )));
                }
            }
        }
        out
    }
}

/// A workflow ready to check content against: its rules, and the custom
/// categories it brings with the embeddings of their prompts. It has no name:
/// whoever holds it names it, and tells the engine when checking.
///
/// It is cheap to clone (the data is shared) and to keep in memory: a
/// workflow without custom category holds only its rules, and one with
/// holds a few kilobytes per prompt. Build it with
/// [`Engine::compile_workflow`](crate::engine::Engine::compile_workflow) or
/// read it back from JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledWorkflow {
    inner: Arc<Compiled>,
}

#[derive(Debug, PartialEq)]
struct Compiled {
    spec: WorkflowSpec,
    concepts: Vec<CompiledConcept>,
}

// A compiled workflow is shared between the threads of a server.
const _: fn() = || {
    fn shared<T: Clone + Send + Sync>() {}
    shared::<CompiledWorkflow>();
};

impl CompiledWorkflow {
    /// A workflow with no custom category: what a configuration file holds.
    pub(crate) fn plain(spec: WorkflowSpec) -> CompiledWorkflow {
        CompiledWorkflow {
            inner: Arc::new(Compiled {
                spec,
                concepts: Vec::new(),
            }),
        }
    }

    /// A workflow with custom categories, each name used once.
    pub(crate) fn new(
        spec: WorkflowSpec,
        concepts: Vec<CompiledConcept>,
    ) -> Result<CompiledWorkflow, Problem> {
        let mut names = BTreeSet::new();
        for concept in &concepts {
            if !names.insert(concept.name.as_str()) {
                return Err(Problem::new(format!(
                    "concept `{}` appears twice",
                    concept.name
                )));
            }
        }
        Ok(CompiledWorkflow {
            inner: Arc::new(Compiled { spec, concepts }),
        })
    }

    /// The rules this workflow checks, image and text.
    pub fn spec(&self) -> &WorkflowSpec {
        &self.inner.spec
    }

    /// The custom categories, with their embeddings.
    pub fn concepts(&self) -> &[CompiledConcept] {
        &self.inner.concepts
    }

    /// The custom category called `name`.
    pub fn concept(&self, name: &str) -> Option<&CompiledConcept> {
        self.inner.concepts.iter().find(|c| c.name == name)
    }
}

/// The version of the stored form that this code reads and writes. A change
/// that older code could misread takes the next number.
const FORMAT: u64 = 1;

// The stored form is `{"format": 1, "spec": ..., "concepts": [...]}`. Each
// concept holds its prompts and negatives with their embeddings, written as
// the little-endian bytes of the f32 values in base64.

#[derive(Serialize)]
struct Stored<'a> {
    format: u64,
    spec: &'a WorkflowSpec,
    concepts: Vec<StoredConcept<'a>>,
}

#[derive(Serialize)]
struct StoredConcept<'a> {
    name: &'a str,
    model: &'a str,
    dimension: usize,
    shift: f32,
    prompts: Vec<StoredPrompt<'a>>,
    negatives: Vec<StoredPrompt<'a>>,
}

#[derive(Serialize)]
struct StoredPrompt<'a> {
    text: &'a str,
    embedding: String,
}

fn stored_prompts(set: &[Embedded]) -> Vec<StoredPrompt<'_>> {
    set.iter()
        .map(|e| StoredPrompt {
            text: &e.text,
            embedding: encode_embedding(&e.embedding),
        })
        .collect()
}

impl Serialize for CompiledWorkflow {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Stored {
            format: FORMAT,
            spec: &self.inner.spec,
            concepts: self
                .inner
                .concepts
                .iter()
                .map(|c| StoredConcept {
                    name: &c.name,
                    model: &c.model,
                    dimension: c.dimension,
                    shift: c.shift,
                    prompts: stored_prompts(&c.prompts),
                    negatives: stored_prompts(&c.negatives),
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

fn encode_embedding(embedding: &[f32]) -> String {
    let bytes: Vec<u8> = embedding.iter().flat_map(|v| v.to_le_bytes()).collect();
    STANDARD.encode(bytes)
}

fn decode_embedding(text: &str) -> Result<Vec<f32>, String> {
    let bytes = STANDARD
        .decode(text)
        .map_err(|e| format!("not valid base64 ({e})"))?;
    let (values, rest) = bytes.as_chunks::<4>();
    if !rest.is_empty() {
        return Err(format!(
            "{} bytes, not a whole number of 32-bit floats",
            bytes.len()
        ));
    }
    Ok(values.iter().map(|v| f32::from_le_bytes(*v)).collect())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadWorkflow {
    // Checked before, on the raw document: here only so that it is allowed.
    #[allow(dead_code)]
    format: u64,
    spec: WorkflowSpec,
    concepts: Vec<ReadConcept>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadConcept {
    name: String,
    model: String,
    dimension: usize,
    shift: f32,
    prompts: Vec<ReadPrompt>,
    negatives: Vec<ReadPrompt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadPrompt {
    text: String,
    embedding: String,
}

impl ReadConcept {
    fn build(self) -> Result<CompiledConcept, Problem> {
        let name = &self.name;
        let read = |set: Vec<ReadPrompt>| -> Result<Vec<(String, Vec<f32>)>, Problem> {
            set.into_iter()
                .map(|p| match decode_embedding(&p.embedding) {
                    Ok(embedding) => Ok((p.text, embedding)),
                    Err(why) => Err(Problem::new(format!(
                        "concept `{name}`: the embedding of {:?} is {why}",
                        p.text
                    ))),
                })
                .collect()
        };
        let prompts = read(self.prompts)?;
        let negatives = read(self.negatives)?;
        CompiledConcept::new(
            &self.name,
            &self.model,
            self.dimension,
            self.shift,
            prompts,
            negatives,
        )
    }
}

impl<'de> Deserialize<'de> for CompiledWorkflow {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        // The format first, so that a document from a later version is
        // refused as such, whatever else it changed.
        let value = serde_json::Value::deserialize(deserializer)?;
        match value.get("format") {
            Some(format) if format.as_u64() == Some(FORMAT) => {}
            Some(format) => {
                return Err(D::Error::custom(format!(
                    "unsupported compiled workflow format {format}: this version of openmod reads format {FORMAT}"
                )));
            }
            None => return Err(D::Error::custom("a compiled workflow needs a `format`")),
        }
        let read: ReadWorkflow = serde_json::from_value(value).map_err(D::Error::custom)?;
        let concepts = read
            .concepts
            .into_iter()
            .map(ReadConcept::build)
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        CompiledWorkflow::new(read.spec, concepts).map_err(D::Error::custom)
    }
}

/// A custom image category: prompts and negatives written in words, with
/// their embeddings by the zero-shot model that encoded them.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledConcept {
    name: String,
    model: String,
    dimension: usize,
    shift: f32,
    prompts: Vec<Embedded>,
    negatives: Vec<Embedded>,
}

/// A prompt and its embedding: a unit vector of the concept's dimension.
#[derive(Clone, PartialEq)]
pub struct Embedded {
    text: String,
    embedding: Vec<f32>,
}

impl fmt::Debug for Embedded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} ({} values)", self.text, self.embedding.len())
    }
}

impl AsRef<[f32]> for Embedded {
    fn as_ref(&self) -> &[f32] {
        &self.embedding
    }
}

impl Embedded {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn embedding(&self) -> &[f32] {
        &self.embedding
    }
}

impl CompiledConcept {
    /// A concept whose every embedding has `dimension` values, all finite,
    /// with at least one prompt: anything else would score wrongly, or never
    /// flag, without a word.
    pub(crate) fn new(
        name: &str,
        model: &str,
        dimension: usize,
        shift: f32,
        prompts: Vec<(String, Vec<f32>)>,
        negatives: Vec<(String, Vec<f32>)>,
    ) -> Result<CompiledConcept, Problem> {
        if prompts.is_empty() {
            return Err(Problem::new(format!("concept `{name}` has no prompt")));
        }
        if dimension == 0 {
            return Err(Problem::new(format!(
                "concept `{name}` declares embeddings of 0 values"
            )));
        }
        let check = |set: Vec<(String, Vec<f32>)>| -> Result<Vec<Embedded>, Problem> {
            set.into_iter()
                .map(|(text, embedding)| {
                    if embedding.len() != dimension {
                        return Err(Problem::new(format!(
                            "concept `{name}`: the embedding of {text:?} holds {} values, the concept declares {dimension}",
                            embedding.len()
                        )));
                    }
                    if embedding.iter().any(|v| !v.is_finite()) {
                        return Err(Problem::new(format!(
                            "concept `{name}`: the embedding of {text:?} holds a NaN or an infinite value"
                        )));
                    }
                    Ok(Embedded { text, embedding })
                })
                .collect()
        };
        Ok(CompiledConcept {
            name: name.to_string(),
            model: model.to_string(),
            dimension,
            shift,
            prompts: check(prompts)?,
            negatives: check(negatives)?,
        })
    }

    /// The category's id, as the workflow's rules name it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The zero-shot model (its name in the configuration) that encoded the
    /// prompts, and the only one that can score this category.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// How many values each embedding holds.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// The logit shift that moves the decision boundary to a score of 0.5.
    pub fn shift(&self) -> f32 {
        self.shift
    }

    pub fn prompts(&self) -> &[Embedded] {
        &self.prompts
    }

    /// What the category is often mistaken for.
    pub fn negatives(&self) -> &[Embedded] {
        &self.negatives
    }
}

/// Why a workflow refuses to check anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// It has no rule for this kind of content.
    NoRules,
    /// None of its rules can be scored: every model they need is disabled
    /// or not ready. Answering "accept" would be a lie.
    NothingCheckable(Vec<String>),
}

impl Workflow {
    /// The rules that cannot be scored with the `available` categories, or
    /// the reason the workflow cannot answer at all.
    pub fn plan(&self, available: &BTreeSet<String>) -> Result<Vec<String>, Unavailable> {
        self.plan_with(&|category| available.contains(category))
    }

    /// Like [`plan`](Workflow::plan), with a test in place of the set: for
    /// the categories that depend on more than a set can say.
    pub fn plan_with(&self, available: &dyn Fn(&str) -> bool) -> Result<Vec<String>, Unavailable> {
        if self.rules.is_empty() {
            return Err(Unavailable::NoRules);
        }
        let unchecked: Vec<String> = self
            .rules
            .keys()
            .filter(|c| !available(c))
            .cloned()
            .collect();
        if unchecked.len() == self.rules.len() {
            return Err(Unavailable::NothingCheckable(unchecked));
        }
        Ok(unchecked)
    }
}
