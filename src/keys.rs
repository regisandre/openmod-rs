//! API keys and the admin token, kept as SHA-256 digests.
//!
//! API keys come from three places: `OPENMOD_API_KEYS`, the `api_keys_file`
//! (one key per line) and the state file the admin interface manages. Only
//! the state file is written by openmod, and it never holds a secret: a
//! generated key or token is shown once, then only its digest is kept. The
//! secrets are random 192-bit values, so a plain SHA-256 digest is enough
//! (a slow password hash protects guessable passwords, not random keys).

use crate::files::write_atomic;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

/// External keys shorter than this are refused: they could be guessed.
pub const MIN_KEY_LEN: usize = 24;

pub fn digest(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.trim().as_bytes()).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// `prefix` followed by 48 random hex digits (192 bits).
pub fn generate(prefix: &str) -> String {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).expect("the operating system random source failed");
    format!("{prefix}{}", hex(&buf))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What the admin interface stores: digests and labels, never secrets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_token_sha256: Option<String>,
    /// Set when the first-run setup is finished or skipped.
    #[serde(default)]
    pub setup_done: bool,
    #[serde(default)]
    pub api_keys: Vec<StoredKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredKey {
    /// Stable id for revocation: the start of the digest.
    pub id: String,
    pub name: String,
    /// The first characters of the key, so its owner can recognize it.
    pub prefix: String,
    pub sha256: String,
    /// Unix seconds.
    pub created: u64,
}

const HEADER: &str =
    "# Managed by openmod. It holds SHA-256 digests only, never a key or a token.\n\n";

impl State {
    /// A missing file is an empty state.
    pub fn load(path: &Path) -> Result<State> {
        match std::fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        write_atomic(path, format!("{HEADER}{body}").as_bytes(), true)
    }

    /// Creates an API key named `name` and returns it. It is never shown again.
    pub fn add_key(&mut self, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            bail!("a key name is 1 to 60 printable characters");
        }
        let key = generate("om_");
        let sha = hex(&digest(&key));
        self.api_keys.push(StoredKey {
            id: sha[..12].to_string(),
            name: name.to_string(),
            prefix: key[..7].to_string(),
            sha256: sha,
            created: now(),
        });
        Ok(key)
    }

    /// Removes the key with this id; false when there is none.
    pub fn revoke(&mut self, id: &str) -> bool {
        let before = self.api_keys.len();
        self.api_keys.retain(|k| k.id != id);
        self.api_keys.len() != before
    }

    /// Replaces the admin token and returns the new one. The old token and
    /// every session opened with it stop working.
    pub fn rotate_admin_token(&mut self) -> String {
        let token = generate("oma_");
        self.admin_token_sha256 = Some(hex(&digest(&token)));
        token
    }

    pub fn admin_digest(&self) -> Option<[u8; 32]> {
        self.admin_token_sha256.as_deref().and_then(unhex)
    }

    pub fn is_admin(&self, token: &str) -> bool {
        self.admin_digest()
            .is_some_and(|d| bool::from(d.ct_eq(&digest(token))))
    }
}

/// Every accepted API key, as digests compared in constant time.
#[derive(Debug, Clone, Default)]
pub struct KeySet {
    digests: Vec<[u8; 32]>,
}

impl KeySet {
    /// Keys one per line or item: blanks and `#` comments are skipped, and
    /// a key shorter than `MIN_KEY_LEN` is refused.
    pub fn from_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Result<KeySet> {
        let mut digests = Vec::new();
        for line in lines
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            if line.len() < MIN_KEY_LEN {
                bail!("API keys must be at least {MIN_KEY_LEN} characters long");
            }
            digests.push(digest(line));
        }
        Ok(KeySet { digests })
    }

    /// `OPENMOD_API_KEYS` (comma separated) and `api_keys_file`.
    pub fn external(api_keys_file: Option<&Path>) -> Result<KeySet> {
        let env = std::env::var("OPENMOD_API_KEYS").unwrap_or_default();
        let mut keys = KeySet::from_lines(env.split(','))?;
        if let Some(path) = api_keys_file {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            keys.digests
                .extend(KeySet::from_lines(raw.lines())?.digests);
        }
        Ok(keys)
    }

    pub fn extend_state(&mut self, state: &State) {
        self.digests
            .extend(state.api_keys.iter().filter_map(|k| unhex(&k.sha256)));
    }

    pub fn len(&self) -> usize {
        self.digests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.digests.is_empty()
    }

    pub fn matches(&self, presented: &str) -> bool {
        let d = digest(presented);
        // Compare with every key so the timing does not tell which matched.
        self.digests
            .iter()
            .fold(false, |ok, k| ok | bool::from(k.ct_eq(&d)))
    }
}
