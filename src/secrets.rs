//! The database passphrase lives in the Secret Service keyring (service `omafinances`, user `db-passphrase`).
//!
//! `import_from_env_file` moves it out of the project's `.env` once. Nothing here ever prints, logs or
//! returns the passphrase except to the caller that opens the database.

use anyhow::{bail, Context, Result};
use std::path::Path;

pub const SERVICE: &str = "omafinances";
pub const DB_PASSPHRASE_USER: &str = "db-passphrase";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, DB_PASSPHRASE_USER).context("open keyring entry")
}

/// The stored passphrase, or `None` when none has been imported yet.
pub fn db_passphrase() -> Result<Option<String>> {
    match entry()?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("keyring read failed: {}", e)),
    }
}

pub fn set_db_passphrase(p: &str) -> Result<()> {
    if p.is_empty() {
        bail!("empty passphrase");
    }
    entry()?.set_password(p).map_err(|e| anyhow::anyhow!("keyring write failed: {}", e))
}

/// Value of `KEY=` in a dotenv-style file (optional surrounding quotes stripped). Never logs the value.
pub fn read_env_value(file: &Path, key: &str) -> Result<Option<String>> {
    let text = std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    Ok(parse_env_value(&text, key))
}

pub fn parse_env_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                let v = v.trim();
                let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))).unwrap_or(v);
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Copy `DB_PASSPHRASE` from `file` into the keyring. Returns whether it replaced an existing entry.
pub fn import_from_env_file(file: &Path) -> Result<bool> {
    let p = read_env_value(file, "DB_PASSPHRASE")?.context("DB_PASSPHRASE not found in the env file")?;
    let existed = db_passphrase()?.is_some();
    set_db_passphrase(&p)?;
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_plain_quoted_and_commented_values() {
        let t = "# c\nA=1\nDB_PASSPHRASE = \"abc=def\"\nB='x'\n";
        assert_eq!(parse_env_value(t, "DB_PASSPHRASE").as_deref(), Some("abc=def"));
        assert_eq!(parse_env_value(t, "B").as_deref(), Some("x"));
        assert_eq!(parse_env_value(t, "A").as_deref(), Some("1"));
        assert_eq!(parse_env_value(t, "NOPE"), None);
        assert_eq!(parse_env_value("#DB_PASSPHRASE=zzz", "DB_PASSPHRASE"), None);
    }
}
