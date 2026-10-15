use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<f32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Workflow {
    #[serde(flatten)]
    pub rules: BTreeMap<String, Threshold>,
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
        if self.rules.is_empty() {
            return Err(Unavailable::NoRules);
        }
        let unchecked: Vec<String> = self
            .rules
            .keys()
            .filter(|c| !available.contains(*c))
            .cloned()
            .collect();
        if unchecked.len() == self.rules.len() {
            return Err(Unavailable::NothingCheckable(unchecked));
        }
        Ok(unchecked)
    }
}
