//! GLiNER2 schema classifier (GLiGuard) as an ONNX graph.
//!
//! GLiNER2 encodes a schema and the text together:
//!
//! ```text
//! ( [P] task ( [L] label [L] label ... ) ) [SEP_STRUCT] ( [P] task2 ... ) [SEP_TEXT] words...
//! ```
//!
//! and scores the encoder state of every `[L]` marker. The graph exported by
//! `tools/export_gliguard.py` takes the token ids and the `[L]` positions and
//! returns one logit per label. The schema part never changes, so its tokens
//! are built once at startup; each check only tokenizes the text.

use crate::config::GlinerTask;
use crate::decision::Score;
use crate::models::onnx::{Runtime, lock, open_session};
use anyhow::{Result, anyhow, bail};
use ort::session::Session;
use ort::value::Tensor;
use regex::Regex;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use tokenizers::Tokenizer;

/// The encoder sees at most 512 tokens.
const MAX_TOKENS: usize = 512;

/// GLiNER2's word splitter: URLs, emails and @mentions stay whole, words may
/// contain `-` or `_`, every other non-space character stands alone. Each
/// piece is tokenized on its own, so a user cannot type a marker such as
/// `[SEP_TEXT]`: it becomes `[`, `sep_text`, `]`.
static WORDS: LazyLock<Regex> = LazyLock::new(|| {
    let w = r"[\p{L}\p{N}_\p{M}\x{200C}\x{200D}]";
    Regex::new(&format!(
        r"(?i)(?:https?://\S+|www\.\S+)|[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{{2,}}|@[a-z0-9_]+|{w}+(?:[\-_]{w}+)*|\S"
    ))
    .unwrap()
});

/// Lowercased words of `text`, which GLiNER2 always ends with punctuation.
pub fn words(text: &str) -> Vec<String> {
    let text = if text.is_empty() {
        ".".to_string()
    } else if text.ends_with(['.', '!', '?']) {
        text.to_string()
    } else {
        format!("{text}.")
    };
    WORDS
        .find_iter(&text)
        .map(|m| m.as_str().to_lowercase())
        .collect()
}

/// The schema pieces for `tasks`, then `[SEP_TEXT]`.
pub fn schema_pieces(tasks: &[GlinerTask]) -> Vec<String> {
    let mut pieces = Vec::new();
    for (i, task) in tasks.iter().enumerate() {
        if i > 0 {
            pieces.push("[SEP_STRUCT]".to_string());
        }
        pieces.extend(["(", "[P]"].map(String::from));
        pieces.push(task.name.clone());
        pieces.push("(".to_string());
        for label in &task.labels {
            pieces.push("[L]".to_string());
            pieces.push(label.clone());
        }
        pieces.extend([")", ")"].map(String::from));
    }
    pieces.push("[SEP_TEXT]".to_string());
    pieces
}

pub struct Gliner {
    name: String,
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    tasks: Vec<GlinerTask>,
    prefix_ids: Vec<i64>,
    label_positions: Vec<i64>,
}

fn encode(tok: &Tokenizer, piece: &str) -> Result<Vec<u32>> {
    Ok(tok
        .encode(piece, false)
        .map_err(|e| anyhow!("tokenizer: {e}"))?
        .get_ids()
        .to_vec())
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl Gliner {
    pub fn load(
        name: &str,
        path: &Path,
        tokenizer: &Path,
        tasks: Vec<GlinerTask>,
        rt: Runtime,
    ) -> Result<Self> {
        let tok =
            Tokenizer::from_file(tokenizer).map_err(|e| anyhow!("{}: {e}", tokenizer.display()))?;
        let marker = encode(&tok, "[L]")?;
        if marker.len() != 1 {
            bail!(
                "{}: `[L]` is not a single token; not a GLiNER2 tokenizer",
                tokenizer.display()
            );
        }
        let mut prefix_ids = Vec::new();
        let mut label_positions = Vec::new();
        for piece in schema_pieces(&tasks) {
            if piece == "[L]" {
                label_positions.push(prefix_ids.len() as i64);
            }
            prefix_ids.extend(encode(&tok, &piece)?.into_iter().map(i64::from));
        }
        if prefix_ids.len() + 16 > MAX_TOKENS {
            bail!(
                "{name}: the schema alone takes {} tokens, too many to leave room for text",
                prefix_ids.len()
            );
        }
        Ok(Gliner {
            name: name.to_string(),
            session: Mutex::new(open_session(path, rt)?),
            tokenizer: tok,
            tasks,
            prefix_ids,
            label_positions,
        })
    }

    /// Token ids of the text, split into chunks that fit next to the schema.
    fn text_chunks(&self, text: &str) -> Result<Vec<Vec<i64>>> {
        let budget = MAX_TOKENS - self.prefix_ids.len();
        let mut chunks = vec![Vec::new()];
        for word in words(text) {
            let ids: Vec<i64> = encode(&self.tokenizer, &word)?
                .into_iter()
                .map(i64::from)
                .collect();
            let current = chunks.last_mut().expect("never empty");
            if !current.is_empty() && current.len() + ids.len() > budget {
                chunks.push(Vec::new());
            }
            let current = chunks.last_mut().expect("never empty");
            current.extend(ids.into_iter().take(budget));
        }
        Ok(chunks)
    }

    fn logits(&self, text_ids: &[i64]) -> Result<Vec<f32>> {
        let mut ids = self.prefix_ids.clone();
        ids.extend_from_slice(text_ids);
        let input = Tensor::from_array(([1usize, ids.len()], ids))?;
        let positions =
            Tensor::from_array(([self.label_positions.len()], self.label_positions.clone()))?;
        let mut session = lock(&self.session);
        let outputs = session.run(ort::inputs!["input_ids" => input, "positions" => positions])?;
        let (_, data) = outputs["logits"].try_extract_tensor::<f32>()?;
        Ok(data.to_vec())
    }

    /// Probability of every label, task by task: a softmax across the labels
    /// of a single-label task, a sigmoid per label of a multi-label task.
    pub fn probabilities(&self, text_ids: &[i64]) -> Result<Vec<Vec<f32>>> {
        let logits = self.logits(text_ids)?;
        let mut out = Vec::with_capacity(self.tasks.len());
        let mut offset = 0;
        for task in &self.tasks {
            let l = &logits[offset..offset + task.labels.len()];
            offset += task.labels.len();
            out.push(if task.multi_label {
                l.iter().map(|&x| sigmoid(x)).collect()
            } else {
                let max = l.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let e: Vec<f32> = l.iter().map(|x| (x - max).exp()).collect();
                let sum: f32 = e.iter().sum();
                e.into_iter().map(|x| x / sum).collect()
            });
        }
        Ok(out)
    }

    /// One score per mapped label, the worst over the text's chunks.
    pub fn run(&self, text: &str) -> Result<Vec<Score>> {
        let mut best: Vec<Vec<f32>> = self
            .tasks
            .iter()
            .map(|t| vec![0.0; t.labels.len()])
            .collect();
        for chunk in self.text_chunks(text)? {
            for (b, p) in best.iter_mut().zip(self.probabilities(&chunk)?) {
                for (x, y) in b.iter_mut().zip(p) {
                    *x = x.max(y);
                }
            }
        }
        let mut out = Vec::new();
        for (task, probs) in self.tasks.iter().zip(best) {
            for (label, p) in task.labels.iter().zip(probs) {
                if let Some(category) = task.categories.get(label) {
                    out.push(Score {
                        category: category.clone(),
                        score: p,
                        source: self.name.clone(),
                    });
                }
            }
        }
        Ok(out)
    }
}
