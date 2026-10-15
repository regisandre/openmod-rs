//! Offline evaluation on a labelled corpus, to calibrate thresholds.

use crate::decision::Action;
use crate::engine::Engine;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

#[derive(Deserialize)]
struct ImageItem {
    path: String,
    category: String,
    expected: String,
}

#[derive(Deserialize)]
struct TextItem {
    id: String,
    lang: String,
    text: String,
    expected: String,
    #[serde(default)]
    categories: Vec<String>,
}

fn parse_action(s: &str) -> Action {
    match s {
        "reject" => Action::Reject,
        "review" => Action::Review,
        _ => Action::Accept,
    }
}

#[derive(Default)]
struct Tally {
    total: usize,
    exact: usize,
    /// Expected reject or review, got accept.
    missed: usize,
    /// Expected accept, got reject.
    false_reject: usize,
    /// Expected accept, got review.
    false_review: usize,
}

impl Tally {
    fn add(&mut self, expected: Action, got: Action) {
        self.total += 1;
        if expected == got {
            self.exact += 1;
        }
        match (expected, got) {
            (Action::Reject | Action::Review, Action::Accept) => self.missed += 1,
            (Action::Accept, Action::Reject) => self.false_reject += 1,
            (Action::Accept, Action::Review) => self.false_review += 1,
            _ => {}
        }
    }
}

fn print_table(title: &str, tallies: &BTreeMap<String, Tally>) {
    println!("\n{title}");
    println!(
        "{:<22} {:>5} {:>7} {:>7} {:>9} {:>9}",
        "group", "n", "exact", "missed", "false_rej", "false_rev"
    );
    let mut all = Tally::default();
    for (k, t) in tallies {
        println!(
            "{:<22} {:>5} {:>6.0}% {:>7} {:>9} {:>9}",
            k,
            t.total,
            100.0 * t.exact as f32 / t.total.max(1) as f32,
            t.missed,
            t.false_reject,
            t.false_review
        );
        all.total += t.total;
        all.exact += t.exact;
        all.missed += t.missed;
        all.false_reject += t.false_reject;
        all.false_review += t.false_review;
    }
    println!(
        "{:<22} {:>5} {:>6.0}% {:>7} {:>9} {:>9}",
        "ALL",
        all.total,
        100.0 * all.exact as f32 / all.total.max(1) as f32,
        all.missed,
        all.false_reject,
        all.false_review
    );
}

fn percentile(v: &mut [u64], p: f32) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f32 * p).round() as usize]
}

/// Runs every item, prints a summary and writes every score to `dump` so
/// thresholds can be tuned without rerunning the models.
pub fn run(
    engine: &Engine,
    images: Option<&Path>,
    texts: Option<&Path>,
    workflow: &str,
    dump: &Path,
) -> Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(dump)?);

    if let Some(manifest) = images {
        let base = manifest.parent().unwrap_or(Path::new("."));
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        let mut times = Vec::new();
        for line in std::fs::read_to_string(manifest)?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let item: ImageItem = serde_json::from_str(line).context("manifest line")?;
            let path = base.join(&item.path);
            let path = if path.exists() {
                path
            } else {
                base.parent().unwrap_or(base).join(&item.path)
            };
            let bytes =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            match engine.check_image(&bytes, workflow) {
                Ok(r) => {
                    times.push(r.time_ms);
                    tallies
                        .entry(item.category.clone())
                        .or_default()
                        .add(parse_action(&item.expected), r.summary.action);
                    let row = serde_json::json!({"kind": "image", "path": item.path, "category": item.category,
                        "expected": item.expected, "action": r.summary.action, "scores": r.scores, "time_ms": r.time_ms});
                    writeln!(out, "{row}")?;
                }
                Err(e) => eprintln!("skip {}: {e}", item.path),
            }
        }
        print_table("IMAGES by category", &tallies);
        println!(
            "latency ms: p50 {} p95 {}",
            percentile(&mut times.clone(), 0.5),
            percentile(&mut times, 0.95)
        );
    }

    if let Some(corpus) = texts {
        let mut by_lang: BTreeMap<String, Tally> = BTreeMap::new();
        let mut by_cat: BTreeMap<String, Tally> = BTreeMap::new();
        let mut times = Vec::new();
        for line in std::fs::read_to_string(corpus)?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let item: TextItem = serde_json::from_str(line).context("corpus line")?;
            let r = engine.check_text(&item.text, workflow)?;
            times.push(r.time_ms);
            let expected = parse_action(&item.expected);
            by_lang
                .entry(item.lang.clone())
                .or_default()
                .add(expected, r.summary.action);
            let group = item
                .categories
                .first()
                .cloned()
                .unwrap_or_else(|| "safe".into());
            by_cat
                .entry(group)
                .or_default()
                .add(expected, r.summary.action);
            let row = serde_json::json!({"kind": "text", "id": item.id, "lang": item.lang, "expected": item.expected,
                "categories": item.categories, "action": r.summary.action, "scores": r.scores, "time_ms": r.time_ms});
            writeln!(out, "{row}")?;
        }
        print_table("TEXT by language", &by_lang);
        print_table("TEXT by expected category", &by_cat);
        println!(
            "latency ms: p50 {} p95 {}",
            percentile(&mut times.clone(), 0.5),
            percentile(&mut times, 0.95)
        );
    }
    println!("\nscores written to {}", dump.display());
    Ok(())
}
