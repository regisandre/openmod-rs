//! ONNX image models: plain classifiers and SigLIP zero-shot concepts.

use crate::config::{ConceptSpec, ImageInput};
use crate::decision::Score;
use crate::image_input::to_tensor;
use crate::pool::Pool;
use anyhow::{Result, anyhow, bail};
use image::RgbImage;
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
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

/// How every model session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Runtime {
    /// CPU threads per model.
    pub threads: usize,
    /// Sessions loaded per model: how many checks run it at once.
    pub sessions: usize,
}

/// `rt.sessions` sessions of one model, for as many checks at once. Each
/// holds its own copy of the weights: sharing ONNX Runtime's prepacked
/// weights between them was measured to save nothing.
pub fn open_sessions(path: &Path, rt: Runtime) -> Result<Pool<Session>> {
    let sessions = (0..rt.sessions.max(1))
        .map(|_| open_session(path, rt))
        .collect::<Result<Vec<_>>>()?;
    Ok(Pool::new(sessions))
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
    sessions: Pool<Session>,
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
            sessions: open_sessions(path, rt)?,
            input,
            classes,
            categories,
            softmax,
        })
    }

    pub fn run(&self, img: &RgbImage) -> Result<Vec<Score>> {
        let s = self.input.size as usize;
        let tensor = Tensor::from_array(([1usize, 3, s, s], to_tensor(img, &self.input)))?;
        let mut session = self.sessions.get();
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

struct Concept {
    category: String,
    shift: f32,
    /// A negative prompt: what the category is often confused with.
    negative: bool,
    embedding: Vec<f32>,
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

/// SigLIP vision tower plus prompt embeddings computed once at startup. The
/// text tower is dropped after that, so it costs memory only while loading.
pub struct ZeroShot {
    name: String,
    sessions: Pool<Session>,
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
        rt: Runtime,
    ) -> Result<Self> {
        let tok = Tokenizer::from_file(tokenizer)
            .map_err(|e| anyhow::anyhow!("{}: {e}", tokenizer.display()))?;
        let mut text_session = open_session(text, rt)?;

        let mut pairs = Vec::new();
        for (category, spec) in concepts {
            for p in spec.prompts() {
                pairs.push((category.clone(), spec.shift(), false, p.clone()));
            }
            for p in spec.negatives() {
                pairs.push((category.clone(), spec.shift(), true, p.clone()));
            }
        }
        let mut ids = Vec::with_capacity(pairs.len() * SIGLIP_TEXT_LEN);
        for (_, _, _, p) in &pairs {
            let enc = tok
                .encode(canonical_prompt(p), true)
                .map_err(|e| anyhow::anyhow!("tokenizing `{p}`: {e}"))?;
            let mut row: Vec<i64> = enc.get_ids().iter().map(|&i| i as i64).collect();
            row.truncate(SIGLIP_TEXT_LEN);
            row.resize(SIGLIP_TEXT_LEN, SIGLIP_PAD_ID);
            ids.extend(row);
        }

        let mut embeddings = Vec::new();
        if !pairs.is_empty() {
            let tensor = Tensor::from_array(([pairs.len(), SIGLIP_TEXT_LEN], ids))?;
            let outputs = text_session.run(ort::inputs!["input_ids" => tensor])?;
            let (shape, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
            let dim = shape[1] as usize;
            for row in data.chunks(dim) {
                let mut v = row.to_vec();
                normalize(&mut v);
                embeddings.push(v);
            }
        }
        drop(text_session);

        let concepts = pairs
            .into_iter()
            .zip(embeddings)
            .map(|((category, shift, negative, _), embedding)| Concept {
                category,
                shift,
                negative,
                embedding,
            })
            .collect();

        Ok(ZeroShot {
            name: name.to_string(),
            sessions: open_sessions(vision, rt)?,
            input,
            scale: logit_scale.exp(),
            bias: logit_bias,
            concepts,
        })
    }

    /// One score per category, see [`concept_score`].
    pub fn run(&self, img: &RgbImage, wanted: &dyn Fn(&str) -> bool) -> Result<Vec<Score>> {
        let s = self.input.size as usize;
        let tensor = Tensor::from_array(([1usize, 3, s, s], to_tensor(img, &self.input)))?;
        let mut image_embedding = {
            let mut session = self.sessions.get();
            let outputs = session.run(ort::inputs!["pixel_values" => tensor])?;
            let (_, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
            data.to_vec()
        };
        normalize(&mut image_embedding);

        // category -> (shift, best prompt similarity, best negative similarity)
        let mut best: BTreeMap<&str, (f32, f32, Option<f32>)> = BTreeMap::new();
        for c in self.concepts.iter().filter(|c| wanted(&c.category)) {
            let cos: f32 = c
                .embedding
                .iter()
                .zip(&image_embedding)
                .map(|(a, b)| a * b)
                .sum();
            let entry = best
                .entry(&c.category)
                .or_insert((c.shift, f32::NEG_INFINITY, None));
            if c.negative {
                entry.2 = Some(entry.2.map_or(cos, |n: f32| n.max(cos)));
            } else {
                entry.1 = entry.1.max(cos);
            }
        }
        Ok(best
            .into_iter()
            .map(|(category, (shift, pos, neg))| Score {
                category: category.to_string(),
                score: concept_score(pos, neg, self.scale, self.bias, shift),
                source: self.name.clone(),
            })
            .collect())
    }

    pub fn categories(&self) -> impl Iterator<Item = &str> {
        self.concepts.iter().map(|c| c.category.as_str())
    }
}
