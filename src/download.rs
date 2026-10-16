//! Model downloads, checked against a pinned size and SHA-256 before use.

use crate::config::Config;
use crate::files::with_suffix;
use crate::tools::{self, File};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Downloads `url` to `dest` and keeps it only if it has exactly `size`
/// bytes and the SHA-256 `sha256`. The bytes go to `dest.part` first, so an
/// interrupted or rejected download never replaces or fakes a model.
pub fn fetch(
    url: &str,
    dest: &Path,
    sha256: &str,
    size: u64,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let name = dest
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(4 * 3600)))
        .user_agent(concat!("openmod/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent
        .get(url)
        .call()
        .with_context(|| format!("downloading {name} from {url}"))?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(size + 1)
        .reader();
    if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let part = with_suffix(dest, ".part");
    let result = (|| -> Result<()> {
        let mut file =
            std::fs::File::create(&part).with_context(|| format!("writing {}", part.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .with_context(|| format!("reading {name}, expected {size} bytes"))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > size {
                bail!("{name} is larger than the expected {size} bytes");
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            progress(done);
        }
        file.sync_all()?;
        if done != size {
            bail!("{name} stopped after {done} of {size} bytes");
        }
        let got: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != sha256 {
            bail!("{name} has SHA-256 {got}, expected {sha256}");
        }
        Ok(())
    })();
    match result {
        Ok(()) => std::fs::rename(&part, dest).with_context(|| format!("moving {name} into place")),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// One file a tool still needs.
#[derive(Debug, Clone, Serialize)]
pub struct Needed {
    pub tool: String,
    pub name: String,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub sha256: String,
    #[serde(skip)]
    pub dest: PathBuf,
    pub size: u64,
}

/// The files the given tools miss, among the ones openmod knows how to
/// download. `tools = None` means every enabled tool.
pub fn needed(config: &Config, tools: Option<&BTreeSet<String>>) -> Vec<Needed> {
    let mut out = Vec::new();
    for (id, entry) in &config.models {
        let wanted = match tools {
            Some(set) => set.contains(id),
            None => entry.enabled,
        };
        if !wanted {
            continue;
        }
        for path in entry.spec.files() {
            if tools::present(path) {
                continue;
            }
            if let Some(file) = tools::file_for(path) {
                out.push(needed_file(id, file, path));
            }
        }
    }
    out
}

fn needed_file(tool: &str, file: &File, dest: &Path) -> Needed {
    Needed {
        tool: tool.to_string(),
        name: file.name.to_string(),
        url: file.url(),
        sha256: file.sha256.to_string(),
        dest: dest.to_path_buf(),
        size: file.size,
    }
}

/// Live state of the downloads started from the admin interface.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Downloads {
    pub running: bool,
    pub files: Vec<Progress>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub tool: String,
    pub name: String,
    pub size: u64,
    pub done: u64,
    /// `queued`, `downloading`, `done` or `failed`.
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Progress {
    pub fn queued(n: &Needed) -> Progress {
        Progress {
            tool: n.tool.clone(),
            name: n.name.clone(),
            size: n.size,
            done: 0,
            state: "queued",
            error: None,
        }
    }
}

/// `openmod fetch`: downloads what the enabled tools miss (every built-in
/// tool with `all`), one file at a time. On a terminal a progress line
/// counts up; in a log (`docker exec`, CI) each file gets one line.
pub fn fetch_command(config: &Config, all: bool) -> Result<()> {
    let live = std::io::stderr().is_terminal();
    let every: BTreeSet<String> = config.models.keys().cloned().collect();
    let list = needed(config, all.then_some(&every));
    if list.is_empty() {
        eprintln!("every model file is present");
        return Ok(());
    }
    let total: u64 = list.iter().map(|n| n.size).sum();
    eprintln!("downloading {} files, {} MB", list.len(), total / 1_000_000);
    for n in &list {
        let mut last = 0;
        fetch(&n.url, &n.dest, &n.sha256, n.size, |done| {
            let pct = done * 100 / n.size.max(1);
            if live && pct != last {
                last = pct;
                eprint!("\r{:<28} {pct:>3}%", n.name);
            }
        })?;
        if live {
            eprintln!("\r{:<28} ok  ", n.name);
        } else {
            eprintln!("{:<28} ok, {} MB", n.name, n.size / 1_000_000);
        }
    }
    Ok(())
}
