//! Where the app keeps its own settings: `~/.config/omafinances/` (0700), `settings.toml` (0600).
//!
//! Settings are TOML and are the only thing stored there besides `build.env`. Financial data stays in the
//! encrypted database; the passphrase stays in the keyring (`secrets`). Nothing secret is ever written here.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "omafinances";
pub const DEFAULT_PORT: u16 = 27183;
/// The existing Lunch Money mirror. Changeable in settings.toml.
pub const DEFAULT_DB: &str = "/mnt/ai/projects/omafinances/finances.db";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub database: DatabaseSettings,
    pub ui: UiSettings,
    pub mcp: McpSettings,
    pub categories: CategorySettings,
    pub mail: MailSettings,
    pub reports: ReportsSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MailSettings {
    /// Path to the Google OAuth "installed app" client JSON. Only the path lives here; the client secret is read
    /// from that file and never copied into settings, logs or errors.
    pub client_secret_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReportsSettings {
    /// Addresses reports may be sent to, in addition to the authenticated Gmail sender (which is always allowed).
    /// Empty = only the sender.
    pub allowed_recipients: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CategorySettings {
    /// Deepest level a category may sit at (top level is 1). Clamped to 1..=6.
    pub max_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DatabaseSettings {
    /// SQLCipher database file (the passphrase is in the keyring, never here).
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiSettings {
    /// Whole-UI scale factor (1.0 = Omarchy font size as is).
    pub scale_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct McpSettings {
    /// Reserved for the MCP chunk. Off by default; it only ever runs while the app is open.
    pub enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            database: DatabaseSettings::default(),
            ui: UiSettings::default(),
            mcp: McpSettings::default(),
            categories: CategorySettings::default(),
            mail: MailSettings::default(),
            reports: ReportsSettings::default(),
        }
    }
}
impl Default for MailSettings {
    fn default() -> Self {
        MailSettings { client_secret_file: String::new() }
    }
}
impl Default for ReportsSettings {
    fn default() -> Self {
        ReportsSettings { allowed_recipients: Vec::new() }
    }
}
impl Default for CategorySettings {
    fn default() -> Self {
        CategorySettings { max_depth: 3 }
    }
}
impl Default for DatabaseSettings {
    fn default() -> Self {
        DatabaseSettings { path: DEFAULT_DB.to_string() }
    }
}
impl Default for UiSettings {
    fn default() -> Self {
        UiSettings { scale_factor: 1.0 }
    }
}
impl Default for McpSettings {
    fn default() -> Self {
        McpSettings { enabled: false }
    }
}

impl Settings {
    /// Clamp values the UI would choke on.
    pub fn sanitized(mut self) -> Self {
        if !(0.5..=3.0).contains(&self.ui.scale_factor) {
            self.ui.scale_factor = 1.0;
        }
        self.categories.max_depth = self.categories.max_depth.clamp(1, 6);
        self
    }
}

/// `$XDG_CONFIG_HOME/omafinances` (or `~/.config/omafinances`), created 0700.
pub fn config_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(dirs::config_dir)
        .context("no config directory")?;
    let dir = base.join(APP_DIR);
    ensure_private_dir(&dir)?;
    Ok(dir)
}

pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.toml")
}

/// Load settings, writing the defaults (0600) when the file is missing. A file that does not parse is an
/// error (reported, never silently replaced: the user's edits are worth more than a clean start).
pub fn load_or_init(config_dir: &Path) -> Result<Settings> {
    let path = settings_path(config_dir);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let s: Settings = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
            Ok(s.sanitized())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let s = Settings::default();
            save(config_dir, &s)?;
            Ok(s)
        }
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Write settings atomically (temp file + rename), 0600.
pub fn save(config_dir: &Path, s: &Settings) -> Result<()> {
    use std::io::Write;
    let path = settings_path(config_dir);
    let tmp = path.with_extension("toml.tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(b"# OmaFinances settings. Secrets are not stored here (database passphrase: Secret Service keyring).\n")?;
    f.write_all(toml::to_string_pretty(s)?.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omafin-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn first_run_writes_private_defaults_and_second_run_reads_them() {
        let d = scratch("cfg");
        ensure_private_dir(&d).unwrap();
        let s = load_or_init(&d).unwrap();
        assert_eq!(s, Settings::default());
        let mode = fs::metadata(settings_path(&d)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(!s.mcp.enabled, "MCP must default to off");
        let mut s2 = s.clone();
        s2.ui.scale_factor = 1.25;
        save(&d, &s2).unwrap();
        assert_eq!(load_or_init(&d).unwrap().ui.scale_factor, 1.25);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn category_max_depth_defaults_to_three_and_is_clamped() {
        let d = scratch("cfg3");
        ensure_private_dir(&d).unwrap();
        assert_eq!(load_or_init(&d).unwrap().categories.max_depth, 3);
        fs::write(settings_path(&d), "[categories]\nmax_depth = 9\n").unwrap();
        assert_eq!(load_or_init(&d).unwrap().categories.max_depth, 6);
        fs::write(settings_path(&d), "[categories]\nmax_depth = 0\n").unwrap();
        assert_eq!(load_or_init(&d).unwrap().categories.max_depth, 1);
        fs::write(settings_path(&d), "[categories]\nmax_depth = 2\n").unwrap();
        assert_eq!(load_or_init(&d).unwrap().categories.max_depth, 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn partial_file_keeps_defaults_and_bad_file_is_an_error_not_a_reset() {
        let d = scratch("cfg2");
        ensure_private_dir(&d).unwrap();
        fs::write(settings_path(&d), "[ui]\nscale_factor = 9.0\n").unwrap();
        let s = load_or_init(&d).unwrap();
        assert_eq!(s.ui.scale_factor, 1.0, "out-of-range scale is clamped");
        assert_eq!(s.database.path, DEFAULT_DB);
        fs::write(settings_path(&d), "this is = = not toml").unwrap();
        assert!(load_or_init(&d).is_err());
        assert!(fs::read_to_string(settings_path(&d)).unwrap().contains("not toml"), "file untouched");
        let _ = fs::remove_dir_all(&d);
    }
}
