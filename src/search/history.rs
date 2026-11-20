//! Search history: the last queries the user ran, newest first, in `<config dir>/search_history.json`
//! (a JSON array of strings, mode 0600, replaced atomically). Query text only, never results.

use anyhow::{Context, Result};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub const MAX_ENTRIES: usize = 50;
/// Longest query accepted, in characters.
pub const MAX_LEN: usize = 500;
pub const FILE: &str = "search_history.json";

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Saved entries, newest first. A missing or unreadable file is an empty history, not an error.
pub fn load(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path(dir)) else {
        return Vec::new();
    };
    let v: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    v.into_iter().take(MAX_ENTRIES).collect()
}

/// Put `q` first, dropping an older identical entry, and keep at most `MAX_ENTRIES`.
pub fn add(dir: &Path, q: &str) -> Result<Vec<String>> {
    let mut v = load(dir);
    v.retain(|e| e != q);
    v.insert(0, q.to_string());
    v.truncate(MAX_ENTRIES);
    save(dir, &v)?;
    Ok(v)
}

/// Temp file (0600, created new) in the same directory, synced, then renamed over the old file.
pub fn save(dir: &Path, entries: &[String]) -> Result<()> {
    crate::config::ensure_private_dir(dir)?;
    let dest = path(dir);
    let tmp = dir.join(format!("{}.tmp{}", FILE, std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let body = serde_json::to_vec(entries)?;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).context("create history temp file")?;
    f.write_all(&body)?;
    f.sync_all()?;
    std::fs::rename(&tmp, &dest).context("replace history file")?;
    Ok(())
}
