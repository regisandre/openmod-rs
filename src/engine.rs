//! Loads the enabled models once and runs only those a workflow needs.

use crate::catalog::{Catalog, Label};
use crate::config::{Config, ModelSpec, QR_CODE, Reader, SecondOpinion, read_category};
use crate::decision::{
    Score, Summary, average_opinions, best_scores, cap_to_verdict, combine_scores, evaluate,
};
use crate::domains::{DomainList, Suffixes};
use crate::image_input::{DecodeError, decode};
use crate::models::gliner::Gliner;
use crate::models::ocr::Ocr;
use crate::models::onnx::{Classifier, Runtime, ZeroShot};
use crate::models::text_guard::TextGuard;
use crate::normalize::deobfuscate;
use crate::tools;
use crate::workflow::{Unavailable, Workflow};
use crate::{domains, extremism, profanity, qr, rules};
use anyhow::{Result, bail};
use image::RgbImage;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

enum ImageModel {
    Classifier(Classifier),
    ZeroShot(ZeroShot),
}

enum TextModel {
    Guard(TextGuard),
    Gliner(Gliner),
}

impl TextModel {
    fn run(&self, text: &str) -> Result<Vec<Score>> {
        match self {
            TextModel::Guard(m) => m.run(text),
            TextModel::Gliner(m) => m.run(text),
        }
    }
}

#[derive(Clone)]
enum Model {
    Image(Arc<ImageModel>),
    Text(Arc<TextModel>),
    List(Arc<DomainList>),
    Qr,
    Ocr(Arc<Ocr>),
}

/// The size and modification time of each file a model reads: a domain list
/// refreshed on disk is loaded again, even with the same settings.
type Stamp = Vec<(u64, Option<SystemTime>)>;

fn stamp(spec: &ModelSpec) -> Stamp {
    spec.files()
        .into_iter()
        .map(|p: &Path| match std::fs::metadata(p) {
            Ok(m) => (m.len(), m.modified().ok()),
            Err(_) => (0, None),
        })
        .collect()
}

/// A loaded model, with the settings it was loaded with so a reload can
/// tell whether it may be kept.
#[derive(Clone)]
struct Slot {
    name: String,
    spec: ModelSpec,
    stamp: Stamp,
    model: Model,
    categories: BTreeSet<String>,
    second_opinion: Option<SecondOpinion>,
}

/// What became of each configured model.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ToolStatus {
    Ready,
    Disabled,
    /// Enabled, but some of its files are not there yet.
    Missing {
        files: Vec<String>,
    },
    /// Enabled, but loading it failed.
    Failed {
        error: String,
    },
}

pub struct Engine {
    pub config: Config,
    catalog: Catalog,
    threads: usize,
    slots: Vec<Slot>,
    pub status: BTreeMap<String, ToolStatus>,
    image_available: BTreeSet<String>,
    text_available: BTreeSet<String>,
    /// Image category -> (reader, text category), for the ready readers.
    derived: BTreeMap<String, (String, String)>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub status: &'static str,
    pub workflow: String,
    pub summary: Summary,
    /// Best score per category among the categories the workflow checks.
    pub scores: BTreeMap<String, f32>,
    /// Workflow categories this server could not check, because every model
    /// that scores them is disabled or not ready.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unchecked: Vec<String>,
    pub time_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("unknown workflow `{0}`")]
    UnknownWorkflow(String),
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the text is longer than {0} characters")]
    TextTooLong(usize),
    #[error("inference failed: {0}")]
    Inference(#[from] anyhow::Error),
}

fn load_model(name: &str, spec: &ModelSpec, rt: Runtime) -> Result<Model> {
    Ok(match spec.clone() {
        ModelSpec::ImageClassifier {
            path,
            input,
            classes,
            categories,
            softmax,
        } => Model::Image(Arc::new(ImageModel::Classifier(Classifier::load(
            name, &path, input, classes, categories, softmax, rt,
        )?))),
        ModelSpec::ImageZeroShot {
            vision,
            text,
            tokenizer,
            input,
            logit_scale,
            logit_bias,
            concepts,
        } => Model::Image(Arc::new(ImageModel::ZeroShot(ZeroShot::load(
            name,
            &vision,
            &text,
            &tokenizer,
            input,
            logit_scale,
            logit_bias,
            &concepts,
            rt,
        )?))),
        ModelSpec::TextGuard {
            path, tokenizer, ..
        } => Model::Text(Arc::new(TextModel::Guard(TextGuard::load(
            name, &path, &tokenizer, rt,
        )?))),
        ModelSpec::TextClassifier {
            path,
            tokenizer,
            tasks,
            ..
        } => Model::Text(Arc::new(TextModel::Gliner(Gliner::load(
            name, &path, &tokenizer, tasks, rt,
        )?))),
        ModelSpec::DomainList {
            paths,
            public_suffixes,
            allow,
            ..
        } => {
            let list = DomainList::load(&paths, &Suffixes::load(&public_suffixes)?)?.allow(&allow);
            if list.is_empty() {
                bail!("{name}: its lists hold no domain");
            }
            tracing::info!(model = %name, domains = list.len(), "domain list loaded");
            Model::List(Arc::new(list))
        }
        ModelSpec::QrReader {} => Model::Qr,
        ModelSpec::ImageOcr {
            detector,
            recognizer,
            dictionary,
            max_side,
            min_score,
        } => Model::Ocr(Arc::new(Ocr::load(
            name,
            &detector,
            &recognizer,
            &dictionary,
            max_side,
            min_score,
            rt,
        )?)),
    })
}

impl Engine {
    /// Loads every enabled model. Never fails: a model whose files are
    /// missing or that fails to load is reported in `status`, and the rules
    /// that need it are unchecked. `previous` lends the models whose
    /// settings did not change, so a reload only loads what is new.
    pub fn load(config: Config, previous: Option<&Engine>) -> Engine {
        let threads = config.server.threads_per_model;
        let rt = Runtime { threads };
        let mut status = BTreeMap::new();
        let mut slots = Vec::new();
        for (name, entry) in &config.models {
            if !entry.enabled {
                status.insert(name.clone(), ToolStatus::Disabled);
                continue;
            }
            let missing: Vec<String> = entry
                .spec
                .files()
                .into_iter()
                .filter(|p| !tools::present(p))
                .map(|p| p.display().to_string())
                .collect();
            if !missing.is_empty() {
                tracing::warn!(model = %name, files = ?missing, "model files missing, not loaded");
                status.insert(name.clone(), ToolStatus::Missing { files: missing });
                continue;
            }
            let files = stamp(&entry.spec);
            let kept = previous
                .filter(|p| p.threads == threads)
                .and_then(|p| {
                    p.slots
                        .iter()
                        .find(|s| s.name == *name && s.spec == entry.spec && s.stamp == files)
                })
                .map(|s| s.model.clone());
            let model = match kept {
                Some(model) => Ok(model),
                None => {
                    tracing::info!(model = %name, "loading");
                    load_model(name, &entry.spec, rt)
                }
            };
            match model {
                Ok(model) => {
                    slots.push(Slot {
                        name: name.clone(),
                        spec: entry.spec.clone(),
                        stamp: files,
                        model,
                        categories: entry.spec.categories(),
                        second_opinion: None,
                    });
                    status.insert(name.clone(), ToolStatus::Ready);
                }
                Err(e) => {
                    tracing::error!(model = %name, error = %format!("{e:#}"), "model failed to load");
                    status.insert(
                        name.clone(),
                        ToolStatus::Failed {
                            error: format!("{e:#}"),
                        },
                    );
                }
            }
        }
        let ready_text: BTreeSet<String> = slots
            .iter()
            .filter(|s| matches!(s.model, Model::Text(_)))
            .map(|s| s.name.clone())
            .collect();
        for (name, so) in config.text_plan(&ready_text) {
            if let Some(slot) = slots.iter_mut().find(|s| s.name == name) {
                slot.second_opinion = so;
            }
        }
        let ready: BTreeSet<&str> = slots.iter().map(|s| s.name.as_str()).collect();
        let keep = |name: &str, _: &crate::config::ModelEntry| ready.contains(name);
        let image_available = config.categories_where(true, &keep);
        let text_available = config.categories_where(false, &keep);
        let derived = config.derived(&keep);
        let mut catalog = Catalog::builtin();
        for (id, l) in &config.labels {
            catalog.set(
                id,
                Label {
                    text: l.text.clone(),
                    message: l.message.clone(),
                },
            );
        }
        let engine = Engine {
            config,
            catalog,
            threads,
            slots,
            status,
            image_available,
            text_available,
            derived,
        };
        if engine
            .status
            .values()
            .any(|s| matches!(s, ToolStatus::Missing { .. }))
        {
            tracing::warn!(
                "download the missing models with `openmod fetch` or from the admin interface"
            );
        }
        let unchecked = engine.unchecked_rules();
        if !unchecked.is_empty() {
            let shown = unchecked
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let more = match unchecked.len().saturating_sub(5) {
                0 => String::new(),
                n => format!(" and {n} more"),
            };
            tracing::warn!(
                "{} workflow rules are not checked, their models are off or not ready: {shown}{more}",
                unchecked.len()
            );
        }
        engine
    }

    /// Enabled models that are not ready, as readable lines.
    pub fn problems(&self) -> Vec<String> {
        self.status
            .iter()
            .filter_map(|(name, s)| match s {
                ToolStatus::Missing { files } => {
                    Some(format!("{name}: missing {}", files.join(", ")))
                }
                ToolStatus::Failed { error } => Some(format!("{name}: {error}")),
                _ => None,
            })
            .collect()
    }

    /// For the command line: every enabled model must be ready.
    pub fn require_ready(&self) -> Result<()> {
        let problems = self.problems();
        if !problems.is_empty() {
            bail!(
                "some enabled models are not ready (run `openmod fetch`):\n  {}",
                problems.join("\n  ")
            );
        }
        Ok(())
    }

    /// Workflow rules no ready model can score, as `workflow.kind.category`.
    pub fn unchecked_rules(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, wf) in &self.config.workflows {
            for (kind, rules, available) in [
                ("image", &wf.image, &self.image_available),
                ("text", &wf.text, &self.text_available),
            ] {
                for category in rules.rules.keys().filter(|c| !available.contains(*c)) {
                    out.push(format!("{name}.{kind}.{category}"));
                }
            }
        }
        out
    }

    /// The title and message of a category, with the configured overrides.
    pub fn label(&self, id: &str) -> Label {
        self.catalog.label(id)
    }

    pub fn available(&self, image: bool) -> &BTreeSet<String> {
        if image {
            &self.image_available
        } else {
            &self.text_available
        }
    }

    /// The second opinion a ready text model gives, if it gives one.
    pub fn second_opinion_of(&self, name: &str) -> Option<&SecondOpinion> {
        self.slots
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.second_opinion.as_ref())
    }

    /// The workflow section for this kind of content, and the rules of it
    /// that cannot be checked.
    fn workflow(&self, name: &str, image: bool) -> Result<(&Workflow, Vec<String>), CheckError> {
        let spec = self
            .config
            .workflows
            .get(name)
            .ok_or_else(|| CheckError::UnknownWorkflow(name.to_string()))?;
        let (wf, kind) = if image {
            (&spec.image, "image")
        } else {
            (&spec.text, "text")
        };
        match wf.plan(self.available(image)) {
            Ok(unchecked) => Ok((wf, unchecked)),
            Err(Unavailable::NoRules) => Err(CheckError::Unavailable(format!(
                "workflow `{name}` has no {kind} rules"
            ))),
            Err(Unavailable::NothingCheckable(categories)) => {
                let needs: Vec<String> = categories
                    .iter()
                    .map(|c| {
                        format!(
                            "`{c}` needs {}",
                            self.config.producers(c, image).join(" or ")
                        )
                    })
                    .collect();
                Err(CheckError::Unavailable(format!(
                    "workflow `{name}` cannot check any {kind} rule on this server: {} (disabled or not ready)",
                    needs.join(", ")
                )))
            }
        }
    }

    fn report(
        &self,
        name: &str,
        wf: &Workflow,
        scores: Vec<Score>,
        unchecked: Vec<String>,
        start: Instant,
    ) -> Report {
        let scores = combine_scores(scores, &self.config.combine);
        let summary = evaluate(wf, &scores, &self.catalog);
        let scores = best_scores(&scores)
            .into_iter()
            .filter(|(c, _)| wf.rules.contains_key(c))
            .map(|(c, s)| (c, s.score))
            .collect();
        Report {
            status: "success",
            workflow: name.to_string(),
            summary,
            scores,
            unchecked,
            time_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Blocking: call from a worker thread.
    pub fn check_image(&self, bytes: &[u8], workflow: &str) -> Result<Report, CheckError> {
        let start = Instant::now();
        let (wf, unchecked) = self.workflow(workflow, true)?;
        let img = decode(bytes, self.config.server.max_image_pixels)?;
        let wanted = |c: &str| wf.rules.contains_key(c);
        let mut scores = Vec::new();
        for slot in &self.slots {
            match &slot.model {
                Model::Image(model) if slot.categories.iter().any(|c| wanted(c)) => {
                    match model.as_ref() {
                        ImageModel::Classifier(c) => scores.extend(c.run(&img)?),
                        ImageModel::ZeroShot(z) => scores.extend(z.run(&img, &wanted)?),
                    }
                }
                Model::Qr | Model::Ocr(_) => scores.extend(self.read_image(slot, &img, &wanted)?),
                _ => {}
            }
        }
        Ok(self.report(workflow, wf, scores, unchecked, start))
    }

    /// What a reader finds in the image, checked like a text: each text
    /// category the workflow wants from it comes back renamed for images
    /// (`qr_link_adult`, `text_extremism`). A reader nothing is wanted from
    /// does not run.
    fn read_image(
        &self,
        slot: &Slot,
        img: &RgbImage,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<Score>, CheckError> {
        let reader = match &slot.model {
            Model::Qr => Reader::Qr,
            Model::Ocr(_) => Reader::Ocr,
            _ => return Ok(Vec::new()),
        };
        let bases: BTreeSet<&str> = self
            .derived
            .iter()
            .filter(|(image, (from, _))| *from == slot.name && wanted(image))
            .map(|(_, (_, text))| text.as_str())
            .collect();
        let wants_code = reader == Reader::Qr && wanted(QR_CODE);
        if bases.is_empty() && !wants_code {
            return Ok(Vec::new());
        }
        let lines = match &slot.model {
            Model::Ocr(ocr) => ocr.read(img)?,
            _ => qr::read(img),
        };
        let mut out = Vec::new();
        if wants_code && !lines.is_empty() {
            out.push(Score {
                category: QR_CODE.to_string(),
                score: 1.0,
                source: slot.name.clone(),
            });
        }
        if lines.is_empty() || bases.is_empty() {
            return Ok(out);
        }
        let max = self.config.server.max_text_chars;
        let text: String = lines.join("\n").chars().take(max).collect();
        let in_text = |c: &str| bases.contains(c);
        let found = match reader {
            Reader::Qr => self.rule_scores(&text, &deobfuscate(&text), &in_text),
            Reader::Ocr => self.text_scores(&text, &in_text)?,
        };
        out.extend(
            found
                .into_iter()
                .filter(|s| in_text(&s.category))
                .map(|s| Score {
                    category: read_category(reader, &s.category),
                    score: s.score,
                    source: format!("{}+{}", slot.name, s.source),
                }),
        );
        Ok(out)
    }

    /// What the rules and the domain lists find in a text. The rules and the
    /// links read the raw text, the word lists the deobfuscated one.
    fn rule_scores(
        &self,
        text: &str,
        model_text: &str,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<Score> {
        let mut scores = rules::scan(text);
        scores.extend(profanity::scan(model_text));
        scores.extend(extremism::scan(text, model_text));
        let lists: Vec<(&Slot, &DomainList)> = self
            .slots
            .iter()
            .filter_map(|s| match &s.model {
                Model::List(list) if s.categories.iter().any(|c| wanted(c)) => {
                    Some((s, list.as_ref()))
                }
                _ => None,
            })
            .collect();
        if lists.is_empty() {
            return scores;
        }
        let hosts = domains::hosts(text);
        for (slot, list) in lists {
            if hosts.iter().any(|h| list.contains(h)) {
                scores.extend(slot.categories.iter().map(|c| Score {
                    category: c.clone(),
                    score: 1.0,
                    source: slot.name.clone(),
                }));
            }
        }
        scores
    }

    /// Everything a text goes through: the rules, the domain lists, and the
    /// text models that score a category the workflow wants.
    fn text_scores(
        &self,
        text: &str,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<Score>, CheckError> {
        // Rules read the raw text; models read it with obfuscation undone.
        let model_text = deobfuscate(text);
        let mut scores = self.rule_scores(text, &model_text, wanted);
        let text_slots = || {
            self.slots.iter().filter_map(|s| match &s.model {
                Model::Text(m) if s.categories.iter().any(|c| wanted(c)) => Some((s, m)),
                _ => None,
            })
        };
        let mut opinions = Vec::new();
        for (_, model) in text_slots().filter(|(s, _)| s.second_opinion.is_none()) {
            opinions.extend(model.run(&model_text)?);
        }
        for (slot, model) in text_slots() {
            let Some(so) = &slot.second_opinion else {
                continue;
            };
            let current = opinions
                .iter()
                .filter(|s| s.category == so.category)
                .map(|s| s.score)
                .fold(0.0f32, f32::max);
            if current >= so.low && current <= so.high {
                opinions = average_opinions(opinions, model.run(&model_text)?);
            }
        }
        scores.extend(cap_to_verdict(opinions, "text_unsafe"));
        Ok(scores)
    }

    /// Blocking: call from a worker thread.
    pub fn check_text(&self, text: &str, workflow: &str) -> Result<Report, CheckError> {
        let start = Instant::now();
        let (wf, unchecked) = self.workflow(workflow, false)?;
        let max = self.config.server.max_text_chars;
        if text.chars().count() > max {
            return Err(CheckError::TextTooLong(max));
        }
        let wanted = |c: &str| wf.rules.contains_key(c);
        let scores = self.text_scores(text, &wanted)?;
        Ok(self.report(workflow, wf, scores, unchecked, start))
    }
}
