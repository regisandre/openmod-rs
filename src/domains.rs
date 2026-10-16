//! Domain lists: whether a link points to a listed site.
//!
//! A list holds one domain per line, in any format blocklists use: a bare
//! domain, a hosts-file line (`0.0.0.0 example.com`) or an adblock rule
//! (`||example.com^`). Each domain is kept as a 64-bit hash, so a million
//! domains take 8 MB.
//!
//! A listed domain covers its subdomains too, except where they belong to
//! someone else:
//! - a public suffix (`tochigi.jp`, a dynamic DNS provider) matches only
//!   itself, as the Public Suffix List defines them;
//! - so does a listed domain with three or more listed subdomains besides
//!   `www` and `m`: it hosts other people's sites (a blog platform), and only
//!   its listed subdomains match.
//!
//! An allowed domain is never flagged, nor are its subdomains: a list is
//! wrong now and then, and a concert hall called a casino is not a casino.

use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// A listed domain with this many listed subdomains hosts other people's sites.
const PLATFORM_SUBDOMAINS: u32 = 3;

/// The most hosts read from one text.
const MAX_HOSTS: usize = 64;

/// FNV-1a over the bytes, then the splitmix64 finalizer, so that names one
/// letter apart land far apart.
fn key(domain: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in domain.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

fn has(sorted: &[u64], k: u64) -> bool {
    sorted.binary_search(&k).is_ok()
}

fn sorted(mut keys: Vec<u64>) -> Vec<u64> {
    keys.sort_unstable();
    keys.dedup();
    keys.shrink_to_fit();
    keys
}

/// The domains above `domain` with at least two labels, nearest first, each
/// with what it leaves out: `a.b.example.com` gives (`a`, `b.example.com`),
/// then (`a.b`, `example.com`).
fn parents(domain: &str) -> impl Iterator<Item = (&str, &str)> {
    domain
        .match_indices('.')
        .map(move |(i, _)| (&domain[..i], &domain[i + 1..]))
        .filter(|(_, parent)| parent.contains('.'))
}

/// `raw` lowercased, when it is a domain name: two labels or more, ASCII
/// letters, digits, `-` and `_`, and not an IPv4 address.
fn clean(raw: &str) -> Option<String> {
    let domain = raw
        .trim()
        .trim_end_matches('.')
        .trim_start_matches("*.")
        .to_ascii_lowercase();
    if domain.len() > 253 || !domain.contains('.') {
        return None;
    }
    let valid = domain.split('.').all(|label| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    let address = domain
        .split('.')
        .all(|label| label.bytes().all(|b| b.is_ascii_digit()));
    (valid && !address).then_some(domain)
}

/// Whether `raw` is a domain name a list or an allowlist can hold.
pub fn is_domain(raw: &str) -> bool {
    clean(raw).is_some()
}

/// Calls `f` with each domain a list line names.
fn line_domains(line: &str, f: &mut dyn FnMut(String)) {
    let line = line.split('#').next().unwrap_or_default().trim();
    if line.is_empty() || line.starts_with('!') {
        return;
    }
    if let Some(rule) = line.strip_prefix("||") {
        let domain = rule.split(['^', '$', '/']).next().unwrap_or_default();
        if let Some(d) = clean(domain) {
            f(d);
        }
        return;
    }
    let mut tokens = line.split_whitespace();
    let Some(first) = tokens.next() else {
        return;
    };
    let mut names = tokens.peekable();
    if names.peek().is_none() {
        if let Some(d) = clean(first) {
            f(d);
        }
    } else if first.parse::<IpAddr>().is_ok() {
        // A hosts-file line: an address, then one or more names.
        names.filter_map(clean).for_each(f);
    }
}

/// The Public Suffix List: the names under which anyone may register a
/// domain of their own.
#[derive(Debug, Default)]
pub struct Suffixes {
    rules: Vec<u64>,
    wildcards: Vec<u64>,
    exceptions: Vec<u64>,
}

impl Suffixes {
    /// Reads the list's own format: one rule per line, `//` comments, `*.`
    /// wildcards and `!` exceptions.
    pub fn parse(text: &str) -> Suffixes {
        let (mut rules, mut wildcards, mut exceptions) = (Vec::new(), Vec::new(), Vec::new());
        for line in text.lines() {
            let rule = line.split_whitespace().next().unwrap_or_default();
            if rule.is_empty() || rule.starts_with("//") {
                continue;
            }
            let rule = rule.to_lowercase();
            if let Some(r) = rule.strip_prefix('!') {
                exceptions.push(key(r));
            } else if let Some(r) = rule.strip_prefix("*.") {
                wildcards.push(key(r));
            } else {
                rules.push(key(&rule));
            }
        }
        Suffixes {
            rules: sorted(rules),
            wildcards: sorted(wildcards),
            exceptions: sorted(exceptions),
        }
    }

    pub fn load(path: &Path) -> Result<Suffixes> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Suffixes::parse(&text))
    }

    /// Whether anyone may register a domain right under `domain`.
    pub fn is_suffix(&self, domain: &str) -> bool {
        if has(&self.exceptions, key(domain)) {
            return false;
        }
        has(&self.rules, key(domain))
            || domain
                .split_once('.')
                .is_some_and(|(_, parent)| has(&self.wildcards, key(parent)))
    }
}

/// One or more domain lists, merged.
#[derive(Debug, Default)]
pub struct DomainList {
    /// Every listed domain.
    domains: Vec<u64>,
    /// The listed domains that match only themselves.
    exact: Vec<u64>,
    /// The domains never flagged, with their subdomains.
    allowed: Vec<u64>,
}

impl DomainList {
    /// A list from list texts held in memory.
    pub fn build(texts: &[&str], suffixes: &Suffixes) -> DomainList {
        let each = |f: &mut dyn FnMut(String)| -> Result<()> {
            for text in texts {
                for line in text.lines() {
                    line_domains(line, f);
                }
            }
            Ok(())
        };
        Self::assemble(each, suffixes).expect("reading memory cannot fail")
    }

    /// A list from list files, read line by line: a 50 MB list never sits
    /// in memory whole.
    pub fn load(paths: &[PathBuf], suffixes: &Suffixes) -> Result<DomainList> {
        let each = |f: &mut dyn FnMut(String)| -> Result<()> {
            for path in paths {
                let file = std::fs::File::open(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let mut reader = BufReader::new(file);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    let n = reader
                        .read_until(b'\n', &mut line)
                        .with_context(|| format!("reading {}", path.display()))?;
                    if n == 0 {
                        break;
                    }
                    line_domains(&String::from_utf8_lossy(&line), f);
                }
            }
            Ok(())
        };
        Self::assemble(each, suffixes)
    }

    /// Two passes over the domains: the first collects them, the second
    /// finds the ones that match only themselves.
    fn assemble(
        each: impl Fn(&mut dyn FnMut(String)) -> Result<()>,
        suffixes: &Suffixes,
    ) -> Result<DomainList> {
        let mut domains = Vec::new();
        each(&mut |d| domains.push(key(&d)))?;
        let domains = sorted(domains);
        let mut exact = Vec::new();
        let mut subdomains: HashMap<u64, u32> = HashMap::new();
        each(&mut |d| {
            if suffixes.is_suffix(&d) {
                exact.push(key(&d));
            }
            for (left, parent) in parents(&d) {
                let k = key(parent);
                if left != "www" && left != "m" && has(&domains, k) {
                    *subdomains.entry(k).or_default() += 1;
                }
            }
        })?;
        exact.extend(
            subdomains
                .into_iter()
                .filter(|(_, n)| *n >= PLATFORM_SUBDOMAINS)
                .map(|(k, _)| k),
        );
        Ok(DomainList {
            domains,
            exact: sorted(exact),
            allowed: Vec::new(),
        })
    }

    /// The same list, never flagging `domains` nor their subdomains. An
    /// entry that is not a domain name allows nothing.
    pub fn allow<S: AsRef<str>>(mut self, domains: &[S]) -> DomainList {
        let keys = domains
            .iter()
            .filter_map(|d| clean(d.as_ref()))
            .map(|d| key(&d));
        self.allowed = sorted(keys.collect());
        self
    }

    /// How many distinct domains the lists hold.
    pub fn len(&self) -> usize {
        self.domains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// Whether `host` is listed, or is under a listed domain that covers its
    /// subdomains, and is neither allowed nor under an allowed domain.
    pub fn contains(&self, host: &str) -> bool {
        let Some(host) = clean(host) else {
            return false;
        };
        let allowed = |d: &str| has(&self.allowed, key(d));
        if allowed(&host) || parents(&host).any(|(_, parent)| allowed(parent)) {
            return false;
        }
        has(&self.domains, key(&host))
            || parents(&host).any(|(_, parent)| {
                let k = key(parent);
                has(&self.domains, k) && !has(&self.exact, k)
            })
    }
}

static HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:[a-z0-9_](?:[a-z0-9_\-]{0,61}[a-z0-9_])?\.)+(?:[a-z]{2,63}|xn--[a-z0-9\-]{1,59})\b")
        .unwrap()
});

/// The hosts a text names, in links or as bare domains, lowercased, at most
/// 64. An email address is not a link to its domain.
pub fn hosts(text: &str) -> Vec<String> {
    let text = text.replace("[.]", ".").replace("(.)", ".");
    let mut out: Vec<String> = Vec::new();
    for m in HOST.find_iter(&text) {
        if text[..m.start()].ends_with('@') || text[m.end()..].starts_with('@') {
            continue;
        }
        let host = m.as_str().to_ascii_lowercase();
        if !out.contains(&host) {
            out.push(host);
        }
        if out.len() == MAX_HOSTS {
            break;
        }
    }
    out
}
