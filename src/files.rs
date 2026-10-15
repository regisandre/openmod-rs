//! File writes that never leave a half-written file behind.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `path` with `suffix` appended to its file name (`a.toml` -> `a.toml.tmp`).
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Writes to a temporary file next to `path`, then renames it over `path`,
/// so a reader sees either the old or the new content. `private` restricts
/// the file to its owner on Unix (for files holding digests of secrets).
pub fn write_atomic(path: &Path, contents: &[u8], private: bool) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = with_suffix(path, ".tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}
