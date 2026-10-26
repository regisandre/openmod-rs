//! Turns detector scores into a decision, following one workflow.

use crate::catalog::Catalog;
use crate::config::Combine;
use crate::workflow::Workflow;
use serde::Serialize;
use std::collections::BTreeMap;

/// One detector's confidence that `category` is present, in `[0, 1]`.
#[derive(Debug, Clone, Serialize)]
pub struct Score {
    pub category: String,
    pub score: f32,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Accept,
    Review,
    Reject,
}

/// Why a piece of content was rejected or sent to review.
#[derive(Debug, Clone, Serialize)]
pub struct Reason {
    /// Stable machine id, e.g. `nudity`. Use it for translations.
    pub id: String,
    /// Short English title, e.g. `Nudity`.
    pub text: String,
    /// Full English sentence that can be shown to an end user.
    pub message: String,
    pub score: f32,
    pub threshold: f32,
    /// The detector that produced the score.
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub action: Action,
    /// Highest score among the rejected categories, 0 when nothing is rejected.
    pub reject_prob: f32,
    pub reject_reason: Vec<Reason>,
    pub review_reason: Vec<Reason>,
}

/// Keeps, for each category, the highest score any detector gave it.
pub fn best_scores(scores: &[Score]) -> BTreeMap<String, Score> {
    let mut best: BTreeMap<String, Score> = BTreeMap::new();
    for s in scores {
        // A score that is not a number compares false with everything: it would replace a real
        // one and slip under every threshold.
        if s.score.is_nan() {
            continue;
        }
        match best.get(&s.category) {
            Some(current) if current.score >= s.score => {}
            _ => {
                best.insert(s.category.clone(), s.clone());
            }
        }
    }
    best
}

/// Applies `[combine]`: the scores of a category configured as `mean` become
/// one score, the mean of each model's best. Every other score is kept, and
/// `best_scores` later picks the highest.
pub fn combine_scores(scores: Vec<Score>, rules: &BTreeMap<String, Combine>) -> Vec<Score> {
    let mean = |c: &str| rules.get(c) == Some(&Combine::Mean);
    // category -> model -> that model's best score, in first-seen model order
    let mut per_model: BTreeMap<String, Vec<(String, f32)>> = BTreeMap::new();
    let mut out = Vec::with_capacity(scores.len());
    for s in scores {
        if !mean(&s.category) {
            out.push(s);
            continue;
        }
        let models = per_model.entry(s.category).or_default();
        match models.iter_mut().find(|(m, _)| *m == s.source) {
            Some((_, best)) => *best = best.max(s.score),
            None => models.push((s.source, s.score)),
        }
    }
    for (category, models) in per_model {
        let score = models.iter().map(|(_, s)| s).sum::<f32>() / models.len() as f32;
        let source = models
            .into_iter()
            .map(|(m, _)| m)
            .collect::<Vec<_>>()
            .join("+");
        out.push(Score {
            category,
            score,
            source,
        });
    }
    out
}

/// Merges a second model's scores into the first ones: a category both
/// score gets the mean of their best scores, any other category is kept as
/// is. Averaging two independent models cuts the confident mistakes each one
/// makes alone.
pub fn average_opinions(first: Vec<Score>, second: Vec<Score>) -> Vec<Score> {
    let a = best_scores(&first);
    let b = best_scores(&second);
    let mut out = Vec::with_capacity(a.len() + b.len());
    for (category, s) in &a {
        out.push(match b.get(category) {
            Some(t) => Score {
                category: category.clone(),
                score: (s.score + t.score) / 2.0,
                source: format!("{}+{}", s.source, t.source),
            },
            None => s.clone(),
        });
    }
    out.extend(
        b.into_iter()
            .filter(|(c, _)| !a.contains_key(c))
            .map(|(_, s)| s),
    );
    out
}

/// Caps every model score at the best `verdict` score (e.g. `text_unsafe`),
/// so a category can name why a text is unsafe but never flag a text the
/// models judge safe overall. Rule hits are exact and stay as they are.
pub fn cap_to_verdict(mut scores: Vec<Score>, verdict: &str) -> Vec<Score> {
    let Some(cap) = scores
        .iter()
        .filter(|s| s.category == verdict)
        .map(|s| s.score)
        .reduce(f32::max)
    else {
        return scores;
    };
    for s in scores
        .iter_mut()
        .filter(|s| s.category != verdict && s.source != "rules")
    {
        s.score = s.score.min(cap);
    }
    scores
}

pub fn evaluate(workflow: &Workflow, scores: &[Score], catalog: &Catalog) -> Summary {
    let best = best_scores(scores);
    let mut reject_reason = Vec::new();
    let mut review_reason = Vec::new();

    for (id, threshold) in &workflow.rules {
        let Some(s) = best.get(id) else { continue };
        let reason = |threshold: f32| {
            let label = catalog.label(id);
            Reason {
                id: id.clone(),
                text: label.text,
                message: label.message,
                score: s.score,
                threshold,
                source: s.source.clone(),
            }
        };
        match (threshold.reject, threshold.review) {
            (Some(t), _) if s.score >= t => reject_reason.push(reason(t)),
            (_, Some(t)) if s.score >= t => review_reason.push(reason(t)),
            _ => {}
        }
    }

    let by_score = |a: &Reason, b: &Reason| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id));
    reject_reason.sort_by(by_score);
    review_reason.sort_by(by_score);

    let action = if !reject_reason.is_empty() {
        Action::Reject
    } else if !review_reason.is_empty() {
        Action::Review
    } else {
        Action::Accept
    };
    let reject_prob = reject_reason.first().map_or(0.0, |r| r.score);

    Summary {
        action,
        reject_prob,
        reject_reason,
        review_reason,
    }
}
