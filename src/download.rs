//! Downloads: model files, checked against a pinned size and SHA-256 before
//! use, and the domain lists, which their publishers keep up to date.

use crate::config::Config;
use crate::files::with_suffix;
use crate::tools::{self, File};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The most a domain list may weigh.
pub const LIST_LIMIT: u64 = 256 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(4 * 3600)))
        .user_agent(concat!("openmod/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn file_name(dest: &Path) -> String {
    dest.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Streams `reader` into `dest.part`, at most `limit` bytes, then lets
/// `check` look at it (byte count, SHA-256) before it replaces `dest`. A
/// download that fails or is refused leaves `dest` as it was and no part
/// file behind.
fn save(
    reader: &mut dyn Read,
    dest: &Path,
    limit: u64,
    progress: &mut dyn FnMut(u64),
    check: impl FnOnce(&Path, u64, &str) -> Result<()>,
) -> Result<()> {
    let name = file_name(dest);
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
                .with_context(|| format!("reading {name}"))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > limit {
                bail!("{name} is larger than the expected {limit} bytes");
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            progress(done);
        }
        file.sync_all()?;
        drop(file);
        let digest: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        check(&part, done, &digest)
    })();
    match result {
        Ok(()) => std::fs::rename(&part, dest).with_context(|| format!("moving {name} into place")),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

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
    let name = file_name(dest);
    let response = agent()
        .get(url)
        .call()
        .with_context(|| format!("downloading {name} from {url}"))?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(size + 1)
        .reader();
    save(&mut reader, dest, size, &mut progress, |_, done, got| {
        if done != size {
            bail!("{name} stopped after {done} of {size} bytes");
        }
        if got != sha256 {
            bail!("{name} has SHA-256 {got}, expected {sha256}");
        }
        Ok(())
    })
}

/// `t` as HTTP writes dates: `Thu, 15 Oct 2026 07:20:42 GMT`.
fn http_date(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let rest = secs % 86_400;
    format!(
        "{}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[days.rem_euclid(7) as usize],
        MONTHS[m as usize - 1],
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The year, month and day of a day count since 1970-01-01 (Howard
/// Hinnant's civil_from_days).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// Whether a list file names at least one entry: a line that is neither
/// blank nor a comment.
fn has_entry(path: &Path) -> Result<bool> {
    let reader = BufReader::new(std::fs::File::open(path)?);
    for line in reader.split(b'\n') {
        let line = line?;
        let line = String::from_utf8_lossy(&line);
        let line = line.trim();
        if !line.is_empty() && !line.starts_with(['#', '!']) && !line.starts_with("//") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Downloads a list its publisher keeps up to date to `dest`, at most
/// `limit` bytes. With a copy already there, the list is asked for only if
/// it changed since that copy was written (`If-Modified-Since`). Returns
/// whether `dest` was replaced. A list with no entry is refused, so a
/// broken publication never leaves a server without its list.
pub fn fetch_list(
    url: &str,
    dest: &Path,
    limit: u64,
    mut progress: impl FnMut(u64),
) -> Result<bool> {
    let name = file_name(dest);
    let copy = std::fs::metadata(dest).and_then(|m| m.modified()).ok();
    let mut request = agent().get(url);
    if let Some(t) = copy {
        request = request.header("If-Modified-Since", http_date(t));
    }
    let response = request
        .call()
        .with_context(|| format!("downloading {name} from {url}"))?;
    if response.status() == 304 {
        // Unchanged: the copy counts as fresh again.
        std::fs::File::options()
            .write(true)
            .open(dest)
            .and_then(|f| f.set_modified(SystemTime::now()))
            .with_context(|| format!("touching {name}"))?;
        return Ok(false);
    }
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(limit + 1)
        .reader();
    save(&mut reader, dest, limit, &mut progress, |part, _, _| {
        if !has_entry(part)? {
            bail!("{name} from {url} has no entry: the copy in place is kept");
        }
        Ok(())
    })?;
    Ok(true)
}

/// One file a tool still needs.
#[derive(Debug, Clone, Serialize)]
pub struct Needed {
    pub tool: String,
    pub name: String,
    #[serde(skip)]
    pub url: String,
    /// Empty for a list, which is not pinned.
    #[serde(skip)]
    pub sha256: String,
    #[serde(skip)]
    pub dest: PathBuf,
    /// An estimate for a list.
    pub size: u64,
}

impl Needed {
    /// Downloads the file: a model checked against its pin, a list as
    /// `fetch_list` takes it.
    pub fn get(&self, progress: impl FnMut(u64)) -> Result<()> {
        if self.sha256.is_empty() {
            fetch_list(&self.url, &self.dest, LIST_LIMIT, progress).map(|_| ())
        } else {
            fetch(&self.url, &self.dest, &self.sha256, self.size, progress)
        }
    }
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
            // Several lists read the same Public Suffix List: one download.
            if tools::present(path) || out.iter().any(|n: &Needed| n.dest == path) {
                continue;
            }
            if let Some(file) = tools::file_for(path) {
                out.push(needed_file(id, file, path));
            }
        }
    }
    out
}

/// Downloads again the lists of the enabled tools written more than
/// `max_age` ago, when they changed upstream. Returns how many were
/// replaced; a list that fails stays as it was, and the error is logged.
pub fn refresh_lists(config: &Config, max_age: Duration) -> usize {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut replaced = 0;
    for entry in config.models.values().filter(|e| e.enabled) {
        for path in entry.spec.files() {
            let Some(file) = tools::file_for(path).filter(|f| !f.pinned()) else {
                continue;
            };
            if !seen.insert(path.to_path_buf()) {
                continue;
            }
            let age = std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok());
            if age.is_none_or(|a| a < max_age) {
                continue;
            }
            match fetch_list(&file.url(), path, LIST_LIMIT, |_| {}) {
                Ok(true) => {
                    tracing::info!(list = file.name, "list updated");
                    replaced += 1;
                }
                Ok(false) => tracing::debug!(list = file.name, "list unchanged"),
                Err(e) => {
                    tracing::warn!(list = file.name, error = %format!("{e:#}"), "list refresh failed, keeping the copy in place")
                }
            }
        }
    }
    replaced
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
        n.get(|done| {
            let pct = (done * 100 / n.size.max(1)).min(100);
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
