//! ONNX image models: plain classifiers and SigLIP zero-shot concepts.

use crate::config::{ConceptSpec, ImageInput};
use crate::decision::Score;
use crate::image_input::to_tensor;
use crate::workflow::CompiledConcept;
use anyhow::{Result, anyhow, bail};
use image::RgbImage;
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use tokenizers::Tokenizer;

static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();

/// The process-wide ONNX Runtime environment, created on first use.
fn environment() -> Result<&'static Environment> {
    if let Some(env) = ENVIRONMENT.get() {
        return Ok(env);
    }
    let env = ort::init()
        .with_name("openmod")
        .build()
        .map_err(|e| anyhow!("onnxruntime: {e}"))?;
    Ok(ENVIRONMENT.get_or_init(|| env))
}

/// Takes a model's session for one call. A check that panicked while it
/// held the session left nothing half-done in it, so its poisoned lock is
/// taken over instead of failing every check after it.
pub fn lock<T>(session: &Mutex<T>) -> MutexGuard<'_, T> {
    session.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How every model session runs.
#[derive(Debug, Clone, Copy)]
pub struct Runtime {
    /// CPU threads per model.
    pub threads: usize,
}

pub fn open_session(path: &Path, rt: Runtime) -> Result<Session> {
    if !path.exists() {
        bail!(
            "model file {} not found (run `openmod fetch`)",
            path.display()
        );
    }
    let err = |e: ort::Error<_>| anyhow!("{}: {e}", path.display());
    // Keep ONNX Runtime's weight prepacking: turning it off was measured at
    // +480 MB and 3x slower, because the 4-bit weights stay unpacked.
    let mut builder = Session::builder(environment()?)
        .map_err(|e| anyhow!("{e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(err)?
        .with_intra_threads(rt.threads)
        .map_err(err)?
        .with_inter_threads(1)
        .map_err(err)?
        // Idle worker threads would otherwise spin and burn CPU between calls.
        .with_intra_op_spinning(false)
        .map_err(err)?
        .with_inter_op_spinning(false)
        .map_err(err)?;
    builder
        .commit_from_file(path)
        .map_err(|e| anyhow!("loading {}: {e}", path.display()))
}

fn softmax(v: &mut [f32]) {
    let max = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for x in v.iter_mut() {
        *x = (*x - max).exp();
        sum += *x;
    }
    for x in v.iter_mut() {
        *x /= sum;
    }
}

pub struct Classifier {
    name: String,
    session: Mutex<Session>,
    input: ImageInput,
    classes: Vec<String>,
    categories: BTreeMap<String, String>,
    softmax: bool,
}

impl Classifier {
    pub fn load(
        name: &str,
        path: &Path,
        input: ImageInput,
        classes: Vec<String>,
        categories: BTreeMap<String, String>,
        softmax: bool,
        rt: Runtime,
    ) -> Result<Self> {
        Ok(Classifier {
            name: name.to_string(),
            session: Mutex::new(open_session(path, rt)?),
            input,
            classes,
            categories,
            softmax,
        })
    }

    pub fn run(&self, img: &RgbImage) -> Result<Vec<Score>> {
        let s = self.input.size as usize;
        let tensor = Tensor::from_array(([1usize, 3, s, s], to_tensor(img, &self.input)))?;
        let mut session = lock(&self.session);
        let input_name = session.inputs()[0].name().to_string();
        let outputs = session.run(ort::inputs![input_name => tensor])?;
        let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
        let mut probs = data.to_vec();
        drop(outputs);
        if probs.len() != self.classes.len() {
            bail!(
                "{}: model returned {} classes, config lists {}",
                self.name,
                probs.len(),
                self.classes.len()
            );
        }
        if self.softmax {
            softmax(&mut probs);
        }
        Ok(self
            .classes
            .iter()
            .zip(probs)
            .filter_map(|(class, p)| {
                self.categories.get(class).map(|category| Score {
                    category: category.clone(),
                    score: p,
                    source: self.name.clone(),
                })
            })
            .collect())
    }
}

/// A zero-shot category of the configuration, with the embeddings of its
/// prompts and of its negatives: what it is often confused with.
struct Concept {
    category: String,
    shift: f32,
    prompts: Vec<Vec<f32>>,
    negatives: Vec<Vec<f32>>,
}

/// The score of a zero-shot category, from the cosine similarity of the image
/// to its best-matching prompt (`pos`) and to its best-matching negative
/// prompt (`neg`), if it has any. SigLIP's own probability for the prompt,
/// moved by `shift`, is multiplied by the probability that the image shows
/// the prompt rather than the negative (a two-way softmax at SigLIP's
/// temperature): an image closer to a negative than to every prompt scores
/// low, whatever its absolute similarity.
pub fn concept_score(pos: f32, neg: Option<f32>, scale: f32, bias: f32, shift: f32) -> f32 {
    let sigmoid = |x: f32| 1.0 / (1.0 + (-x).exp());
    let p = sigmoid(pos * scale + bias + shift);
    match neg {
        Some(n) => p * sigmoid((pos - n) * scale),
        None => p,
    }
}

/// An embedding that does not have the size of the image's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an embedding holds {found} values, the image's holds {expected}")]
pub struct DimensionMismatch {
    pub expected: usize,
    pub found: usize,
}

/// The score of a zero-shot category from the embeddings of an image and of
/// the prompts and negatives of the category, see [`concept_score`] for the
/// formula: the best prompt and the best negative are the ones most similar
/// to the image. All embeddings are unit vectors (the cosine similarity is
/// their dot product) and `scale` is `exp(logit_scale)`. A category without
/// prompt scores 0. An embedding that does not have the image's size is
/// refused, never read as far as it goes.
pub fn score_from_embeddings<E: AsRef<[f32]>>(
    image: &[f32],
    prompts: &[E],
    negatives: &[E],
    scale: f32,
    bias: f32,
    shift: f32,
) -> Result<f32, DimensionMismatch> {
    let best = |set: &[E]| -> Result<Option<f32>, DimensionMismatch> {
        let mut best: Option<f32> = None;
        for embedding in set {
            let embedding = embedding.as_ref();
            if embedding.len() != image.len() {
                return Err(DimensionMismatch {
                    expected: image.len(),
                    found: embedding.len(),
                });
            }
            let cos: f32 = embedding.iter().zip(image).map(|(a, b)| a * b).sum();
            best = Some(best.map_or(cos, |b| b.max(cos)));
        }
        Ok(best)
    };
    let pos = best(prompts)?.unwrap_or(f32::NEG_INFINITY);
    Ok(concept_score(pos, best(negatives)?, scale, bias, shift))
}

/// The text half of SigLIP, kept loaded to encode prompts after startup.
struct TextEncoder {
    tokenizer: Tokenizer,
    session: Mutex<Session>,
}

/// SigLIP vision tower plus prompt embeddings computed once at startup. The
/// text tower is dropped after that, so it costs memory only while loading,
/// unless `keep_text_encoder` keeps it to encode more prompts later.
pub struct ZeroShot {
    name: String,
    session: Mutex<Session>,
    text: Option<Box<TextEncoder>>,
    input: ImageInput,
    scale: f32,
    bias: f32,
    concepts: Vec<Concept>,
}

/// SigLIP's tokenizer lowercases and drops punctuation.
fn canonical_prompt(p: &str) -> String {
    let cleaned: String = p
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    for x in v.iter_mut() {
        *x /= n;
    }
}

const SIGLIP_TEXT_LEN: usize = 64;
const SIGLIP_PAD_ID: i64 = 1;

/// Encodes prompts with SigLIP's text half, all in one batch: one unit vector
/// each. The int8 model scales its activations once per batch, so what a
/// prompt gives depends on the prompts encoded with it, enough to move a
/// borderline score by a few hundredths. Whoever encodes a category must
/// therefore pick its batch with care: the configuration encodes all of its
/// categories together, and a compiled workflow each category on its own.
fn encode_prompts(
    tokenizer: &Tokenizer,
    session: &mut Session,
    prompts: &[&str],
) -> Result<Vec<Vec<f32>>> {
    if prompts.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::with_capacity(prompts.len() * SIGLIP_TEXT_LEN);
    for p in prompts {
        let enc = tokenizer
            .encode(canonical_prompt(p), true)
            .map_err(|e| anyhow!("tokenizing `{p}`: {e}"))?;
        let mut row: Vec<i64> = enc.get_ids().iter().map(|&i| i as i64).collect();
        row.truncate(SIGLIP_TEXT_LEN);
        row.resize(SIGLIP_TEXT_LEN, SIGLIP_PAD_ID);
        ids.extend(row);
    }
    let tensor = Tensor::from_array(([prompts.len(), SIGLIP_TEXT_LEN], ids))?;
    let outputs = session.run(ort::inputs!["input_ids" => tensor])?;
    let (shape, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
    let dim = shape[1] as usize;
    Ok(data
        .chunks(dim)
        .map(|row| {
            let mut v = row.to_vec();
            normalize(&mut v);
            v
        })
        .collect())
}

impl ZeroShot {
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        name: &str,
        vision: &Path,
        text: &Path,
        tokenizer: &Path,
        input: ImageInput,
        logit_scale: f32,
        logit_bias: f32,
        concepts: &BTreeMap<String, ConceptSpec>,
        keep_text_encoder: bool,
        rt: Runtime,
    ) -> Result<Self> {
        let tok =
            Tokenizer::from_file(tokenizer).map_err(|e| anyhow!("{}: {e}", tokenizer.display()))?;
        let mut text_session = open_session(text, rt)?;

        // Every prompt of the configuration in one batch, each category's
        // prompts then its negatives.
        let mut prompts: Vec<&str> = Vec::new();
        for spec in concepts.values() {
            prompts.extend(
                spec.prompts()
                    .iter()
                    .chain(spec.negatives())
                    .map(String::as_str),
            );
        }
        let mut embeddings = encode_prompts(&tok, &mut text_session, &prompts)?.into_iter();
        let concepts = concepts
            .iter()
            .map(|(category, spec)| Concept {
                category: category.clone(),
                shift: spec.shift(),
                prompts: embeddings.by_ref().take(spec.prompts().len()).collect(),
                negatives: embeddings.by_ref().take(spec.negatives().len()).collect(),
            })
            .collect();

        // Freed before the vision half loads, as it always was, unless the
        // configuration asks to keep it.
        let text = if keep_text_encoder {
            Some(Box::new(TextEncoder {
                tokenizer: tok,
                session: Mutex::new(text_session),
            }))
        } else {
            drop(text_session);
            None
        };
        Ok(ZeroShot {
            name: name.to_string(),
            session: Mutex::new(open_session(vision, rt)?),
            text,
            input,
            scale: logit_scale.exp(),
            bias: logit_bias,
            concepts,
        })
    }

    /// Whether the text half of the model is still loaded, so that prompts can
    /// be encoded now (`keep_text_encoder`).
    pub fn can_encode(&self) -> bool {
        self.text.is_some()
    }

    /// Encodes prompts into unit vectors, in one batch. Needs the text half of
    /// the model, which `keep_text_encoder = true` keeps loaded.
    pub fn encode(&self, prompts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let Some(text) = &self.text else {
            bail!(
                "{}: its text encoder was freed after startup; set `keep_text_encoder = true` to keep it",
                self.name
            );
        };
        encode_prompts(&text.tokenizer, &mut lock(&text.session), prompts)
    }

    /// The unit embedding of an image: one pass of the vision half.
    pub fn embed(&self, img: &RgbImage) -> Result<Vec<f32>> {
        let s = self.input.size as usize;
        let tensor = Tensor::from_array(([1usize, 3, s, s], to_tensor(img, &self.input)))?;
        let mut embedding = {
            let mut session = lock(&self.session);
            let outputs = session.run(ort::inputs!["pixel_values" => tensor])?;
            let (_, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
            data.to_vec()
        };
        normalize(&mut embedding);
        Ok(embedding)
    }

    /// One score per category of the configuration that `wanted` accepts,
    /// from the embedding of an image, see [`score_from_embeddings`].
    pub fn score(
        &self,
        image_embedding: &[f32],
        wanted: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<Score>> {
        self.concepts
            .iter()
            .filter(|c| wanted(&c.category))
            .map(|c| {
                Ok(Score {
                    category: c.category.clone(),
                    score: score_from_embeddings(
                        image_embedding,
                        &c.prompts,
                        &c.negatives,
                        self.scale,
                        self.bias,
                        c.shift,
                    )?,
                    source: self.name.clone(),
                })
            })
            .collect()
    }

    /// The score of a custom category, from the embedding of an image. The
    /// category must have been encoded by this model.
    pub fn score_compiled(
        &self,
        image_embedding: &[f32],
        concept: &CompiledConcept,
    ) -> Result<Score, DimensionMismatch> {
        Ok(Score {
            category: concept.name().to_string(),
            score: score_from_embeddings(
                image_embedding,
                concept.prompts(),
                concept.negatives(),
                self.scale,
                self.bias,
                concept.shift(),
            )?,
            source: self.name.clone(),
        })
    }

    /// One score per category of the configuration, see [`concept_score`].
    pub fn run(&self, img: &RgbImage, wanted: &dyn Fn(&str) -> bool) -> Result<Vec<Score>> {
        self.score(&self.embed(img)?, wanted)
    }

    pub fn categories(&self) -> impl Iterator<Item = &str> {
        self.concepts.iter().map(|c| c.category.as_str())
    }
}
