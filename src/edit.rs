//! Edits `openmod.toml` in place for the admin interface: only the part that
//! changes, keeping the comments and layout around it. An edit that would
//! make the file invalid is refused before anything is written.

use crate::config::{Config, DEFAULT_CONFIG};
use crate::files::write_atomic;
use crate::workflow::Threshold;
use anyhow::{Context, Result, anyhow, bail};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use toml_edit::{DocumentMut, InlineTable, Item, Key, Table, Value};

#[derive(Debug, Clone)]
pub enum Edit {
    /// Turn a tool on or off. Turning on a built-in tool missing from the
    /// file adds its block from the default configuration.
    SetTool {
        id: String,
        enabled: bool,
    },
    /// Replace the rules of a workflow, creating it if needed.
    SetWorkflow {
        name: String,
        image: BTreeMap<String, Threshold>,
        text: BTreeMap<String, Threshold>,
    },
    DeleteWorkflow {
        name: String,
    },
}

/// Applies `edit` to the configuration text `raw` and returns the new text,
/// validated.
pub fn apply_to_str(raw: &str, edit: &Edit) -> Result<String> {
    let mut doc: DocumentMut = raw.parse().context("parsing the configuration")?;
    match edit {
        Edit::SetTool { id, enabled } => set_tool(&mut doc, id, *enabled)?,
        Edit::SetWorkflow { name, image, text } => set_workflow(&mut doc, name, image, text)?,
        Edit::DeleteWorkflow { name } => delete_workflow(&mut doc, name)?,
    }
    let out = doc.to_string();
    Config::parse(&out, Path::new("."))
        .context("the change would make the configuration invalid")?;
    Ok(out)
}

/// Applies `edit` to the file at `path` and returns the configuration it now
/// holds. The file is replaced atomically, never left half-written.
pub fn apply_file(path: &Path, edit: &Edit) -> Result<Config> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let out = apply_to_str(&raw, edit)?;
    write_atomic(path, out.as_bytes(), false)?;
    Config::load(path)
}

fn table<'a>(parent: &'a mut Table, key: &str, implicit: bool) -> Result<&'a mut Table> {
    let item = parent.entry(key).or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(implicit);
        Item::Table(t)
    });
    item.as_table_mut()
        .ok_or_else(|| anyhow!("`{key}` is not a table"))
}

fn set_tool(doc: &mut DocumentMut, id: &str, enabled: bool) -> Result<()> {
    let models = table(doc.as_table_mut(), "models", true)?;
    if !models.contains_key(id) {
        let default: DocumentMut = DEFAULT_CONFIG.parse()?;
        let Some(mut block) = default.get("models").and_then(|m| m.get(id)).cloned() else {
            bail!("unknown tool `{id}`");
        };
        if !enabled {
            return Ok(());
        }
        // Without a position, the block is written right after the last model.
        if let Some(t) = block.as_table_mut() {
            clear_positions(t);
        }
        models.insert(id, block);
    }
    let model = models
        .get_mut(id)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("`models.{id}` is not a table"))?;
    let fresh = !model.contains_key("enabled");
    model.insert("enabled", toml_edit::value(enabled));
    if fresh {
        // Keep `enabled` next to `kind`, where a reader looks for it.
        let rank = |k: &Key| match k.get() {
            "kind" => 0,
            "enabled" => 1,
            _ => 2,
        };
        model.sort_values_by(|a, _, b, _| rank(a).cmp(&rank(b)).then(Ordering::Equal));
    }
    Ok(())
}

fn clear_positions(t: &mut Table) {
    t.set_position(None);
    for (_, item) in t.iter_mut() {
        match item {
            Item::Table(sub) => clear_positions(sub),
            Item::ArrayOfTables(array) => array.iter_mut().for_each(clear_positions),
            _ => {}
        }
    }
}

/// A workflow name is what a client sends in a URL or a form field: keep it
/// a plain identifier.
pub fn valid_workflow_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Thresholds come in as `f32`: write them rounded, `0.3` and not
/// `0.30000001192092896`.
fn rounded(x: f32) -> f64 {
    (f64::from(x) * 1000.0).round() / 1000.0
}

fn threshold_value(t: &Threshold) -> Value {
    let mut inline = InlineTable::new();
    if let Some(v) = t.reject {
        inline.insert("reject", Value::from(rounded(v)));
    }
    if let Some(v) = t.review {
        inline.insert("review", Value::from(rounded(v)));
    }
    inline.fmt();
    Value::InlineTable(inline)
}

fn set_workflow(
    doc: &mut DocumentMut,
    name: &str,
    image: &BTreeMap<String, Threshold>,
    text: &BTreeMap<String, Threshold>,
) -> Result<()> {
    if !valid_workflow_name(name) {
        bail!("a workflow name is 1 to 40 characters among a-z, 0-9, `_` and `-`, got `{name}`");
    }
    let workflows = table(doc.as_table_mut(), "workflows", true)?;
    let workflow = table(workflows, name, true)?;
    for (kind, rules) in [("image", image), ("text", text)] {
        if rules.is_empty() {
            workflow.remove(kind);
            continue;
        }
        let section = table(workflow, kind, false)?;
        let stale: Vec<String> = section
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| !rules.contains_key(k))
            .collect();
        for key in stale {
            section.remove(&key);
        }
        for (category, threshold) in rules {
            let mut value = threshold_value(threshold);
            match section.get_mut(category) {
                Some(item) => {
                    // Keep the spacing and the trailing comment of the old value.
                    if let Some(old) = item.as_value() {
                        *value.decor_mut() = old.decor().clone();
                    }
                    *item = Item::Value(value);
                }
                None => {
                    section.insert(category, Item::Value(value));
                }
            }
        }
    }
    // A workflow without rules still exists: write its `[workflows.<name>]`.
    let empty = !workflow.contains_key("image") && !workflow.contains_key("text");
    workflow.set_implicit(!empty);
    Ok(())
}

fn delete_workflow(doc: &mut DocumentMut, name: &str) -> Result<()> {
    if name == "default" {
        bail!("the `default` workflow cannot be deleted, only edited");
    }
    let removed = doc
        .get_mut("workflows")
        .and_then(Item::as_table_mut)
        .and_then(|w| w.remove(name));
    if removed.is_none() {
        bail!("no workflow named `{name}`");
    }
    Ok(())
}
