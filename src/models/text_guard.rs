//! Qwen3Guard-Gen as an ONNX graph, run with ONNX Runtime.
//!
//! The fixed prompt head (the safety policy) is baked into the graph as a
//! precomputed key/value cache by `tools/export_qwen3guard.py`, so each check
//! only processes the user text and the fixed tail. The graph returns the
//! logits at two positions, read in a single forward pass over
//! `text + tail + " Unsafe\nCategories:"`:
//!
//! - after `Safety:`: the Safe / Unsafe / Controversial distribution;
//! - after `Categories:`: which category an unsafe verdict would name first.

use crate::config::TEXT_GUARD_CATEGORIES;
use crate::decision::Score;
use crate::models::onnx::{Runtime, open_sessions};
use crate::pool::Pool;
use anyhow::{Result, anyhow, bail};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use tokenizers::Tokenizer;

/// Everything after the user text, up to the label. Must match the chat
/// template of Qwen3Guard-Gen for a single user message.
const PROMPT_TAIL: &str = "\n\n<END CONVERSATION>\n\nProvide your safety assessment for ONLY THE LAST **USER's query** in the above conversation:\n - The first line must be one of: 'Safety: Safe', 'Safety: Unsafe', 'Safety: Controversial'.\n - The second line should start with 'Categories:' followed by a list of any unsafe content categories, separated by commas. If the content is safe, use 'Categories: None'.<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\nSafety:";

const CATEGORY_PROBE: &str = " Unsafe\nCategories:";

/// Longest piece of text sent in one pass; longer texts are split.
const CHUNK_CHARS: usize = 1500;

pub struct TextGuard {
    name: String,
    sessions: Pool<Session>,
    tokenizer: Tokenizer,
    /// Strings the tokenizer treats as control tokens; user text must never
    /// contain them verbatim.
    control: Vec<String>,
    tail_ids: Vec<u32>,
    probe_ids: Vec<u32>,
    /// First tokens of " Safe", " Unsafe", " Controversial".
    label_ids: [u32; 3],
    /// First token of each category name, with its openmod id (`None` last).
    category_ids: Vec<(u32, Option<&'static str>)>,
}

fn encode(tok: &Tokenizer, s: &str) -> Result<Vec<u32>> {
    Ok(tok
        .encode(s, false)
        .map_err(|e| anyhow!("tokenizer: {e}"))?
        .get_ids()
        .to_vec())
}

fn first(tok: &Tokenizer, s: &str) -> Result<u32> {
    encode(tok, s)?
        .first()
        .copied()
        .ok_or_else(|| anyhow!("empty tokenization of {s:?}"))
}

/// Splits on character boundaries into chunks of at most `max` characters.
pub fn chunks(text: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (i, _) in text.char_indices() {
        if count == max {
            out.push(&text[start..i]);
            start = i;
            count = 0;
        }
        count += 1;
    }
    if start < text.len() || out.is_empty() {
        out.push(&text[start..]);
    }
    out
}

/// Breaks every control-token string in user text with a zero-width space,
/// so a user cannot close the conversation block and answer for the model.
pub fn neutralize(text: &str, control: &[String]) -> String {
    let mut s = text.to_string();
    for c in control {
        if c.len() > 1 && s.contains(c.as_str()) {
            let mut chars = c.chars();
            let head = chars.next().unwrap_or_default();
            s = s.replace(c.as_str(), &format!("{head}\u{200B}{}", chars.as_str()));
        }
    }
    s
}

fn softmax_of(logits: &[f32], ids: impl Iterator<Item = u32>) -> Vec<f32> {
    let picked: Vec<f32> = ids.map(|i| logits[i as usize]).collect();
    let max = picked.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = picked.iter().map(|l| (l - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    exps.into_iter().map(|e| e / sum).collect()
}

/// The model's reading of one text.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub safe: f32,
    pub unsafe_: f32,
    pub controversial: f32,
    /// Probability of each category being named first, given "Unsafe".
    pub categories: Vec<(&'static str, f32)>,
}

impl TextGuard {
    pub fn load(name: &str, path: &Path, tokenizer: &Path, rt: Runtime) -> Result<Self> {
        let tok =
            Tokenizer::from_file(tokenizer).map_err(|e| anyhow!("{}: {e}", tokenizer.display()))?;
        let label_ids = [
            first(&tok, " Safe")?,
            first(&tok, " Unsafe")?,
            first(&tok, " Controversial")?,
        ];
        if label_ids[0] == label_ids[1]
            || label_ids[1] == label_ids[2]
            || label_ids[0] == label_ids[2]
        {
            bail!("the three Qwen3Guard labels share a first token; unsupported tokenizer");
        }
        let mut category_ids = Vec::new();
        for (label, id) in TEXT_GUARD_CATEGORIES {
            category_ids.push((first(&tok, &format!(" {label}"))?, Some(*id)));
        }
        category_ids.push((first(&tok, " None")?, None));
        let mut seen: Vec<u32> = category_ids.iter().map(|(t, _)| *t).collect();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != category_ids.len() {
            bail!("two Qwen3Guard categories share a first token; unsupported tokenizer");
        }
        let control = tok
            .get_added_tokens_decoder()
            .values()
            .map(|t| t.content.clone())
            .collect();
        Ok(TextGuard {
            name: name.to_string(),
            sessions: open_sessions(path, rt)?,
            tail_ids: encode(&tok, PROMPT_TAIL)?,
            probe_ids: encode(&tok, CATEGORY_PROBE)?,
            tokenizer: tok,
            control,
            label_ids,
            category_ids,
        })
    }

    pub fn verdict(&self, text: &str) -> Result<Verdict> {
        // The baked head ends with `USER:`; the text starts after one space.
        let mut ids = encode(
            &self.tokenizer,
            &format!(" {}", neutralize(text, &self.control)),
        )?;
        ids.extend(&self.tail_ids);
        let label_pos = ids.len() - 1;
        ids.extend(&self.probe_ids);
        let last = ids.len() - 1;

        let input = Tensor::from_array((
            [1usize, ids.len()],
            ids.iter().map(|&i| i as i64).collect::<Vec<_>>(),
        ))?;
        let positions = Tensor::from_array(([2usize], vec![label_pos as i64, last as i64]))?;
        let mut session = self.sessions.get();
        let outputs = session.run(ort::inputs!["input_ids" => input, "positions" => positions])?;
        let (shape, data) = outputs["logits"].try_extract_tensor::<f32>()?;
        let vocab = shape[1] as usize;
        let (at_label, at_category) = data.split_at(vocab);

        let labels = softmax_of(at_label, self.label_ids.iter().copied());
        let cats = softmax_of(at_category, self.category_ids.iter().map(|(t, _)| *t));
        Ok(Verdict {
            safe: labels[0],
            unsafe_: labels[1],
            controversial: labels[2],
            categories: self
                .category_ids
                .iter()
                .zip(cats)
                .filter_map(|((_, id), p)| id.map(|id| (id, p)))
                .collect(),
        })
    }

    /// Scores a text, chunk by chunk, keeping the worst chunk. A category's
    /// score is the probability that the text is not safe, times the
    /// probability that this category is the one named.
    pub fn run(&self, text: &str) -> Result<Vec<Score>> {
        let mut worst: Option<Verdict> = None;
        for chunk in chunks(text, CHUNK_CHARS) {
            let v = self.verdict(chunk)?;
            if worst.as_ref().is_none_or(|w| v.safe < w.safe) {
                worst = Some(v);
            }
        }
        let Some(v) = worst else { return Ok(vec![]) };
        let risk = 1.0 - v.safe;
        let score = |category: &str, s: f32| Score {
            category: category.to_string(),
            score: s,
            source: self.name.clone(),
        };
        let mut out = vec![score("text_unsafe", v.unsafe_ + 0.5 * v.controversial)];
        out.extend(v.categories.iter().map(|(id, p)| score(id, risk * p)));
        Ok(out)
    }
}
