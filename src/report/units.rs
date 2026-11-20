//! systemd user timers for enabled reports. Every `systemctl` call goes through `Systemd`, so tests use a fake and
//! write into a temp directory (`OMAFINANCES_SYSTEMD_DIR`).

use super::schedule::Schedule;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PREFIX: &str = "omafinances-report-";
pub const INSTALLED_BINARY: &str = "/opt/omafinances/bin/omafinances";

pub trait Systemd {
    /// Run `systemctl --user <args…>`. The error carries no output (systemctl can echo unit descriptions).
    fn user(&self, args: &[&str]) -> Result<(), String>;
}

pub struct RealSystemd;

impl Systemd for RealSystemd {
    fn user(&self, args: &[&str]) -> Result<(), String> {
        let out = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|_| "systemctl is not available".to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("systemctl --user {} failed", args.first().copied().unwrap_or("")))
        }
    }
}

pub fn service_file(id: i64) -> String {
    format!("{PREFIX}{id}.service")
}

pub fn timer_file(id: i64) -> String {
    format!("{PREFIX}{id}.timer")
}

/// `OMAFINANCES_SYSTEMD_DIR`, else `$XDG_CONFIG_HOME/systemd/user`, else `~/.config/systemd/user`.
pub fn default_units_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OMAFINANCES_SYSTEMD_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("systemd").join("user")
}

/// The installed binary when it exists, else the running one.
pub fn default_binary() -> PathBuf {
    pick_binary(Path::new(INSTALLED_BINARY), std::env::current_exe().ok())
}

pub fn pick_binary(installed: &Path, exe: Option<PathBuf>) -> PathBuf {
    if installed.is_file() {
        return installed.to_path_buf();
    }
    exe.unwrap_or_else(|| PathBuf::from("omafinances"))
}

/// One systemd word: plain paths stay bare; anything else is double-quoted with `\`, `"` and `$`/`%` escaped.
pub fn exec_word(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':' | '@' | '='));
    if plain {
        return s.to_string();
    }
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("$$"),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn render_service(id: i64, binary: &Path) -> String {
    format!(
        "[Unit]\nDescription=OmaFinances report {id}\n\n[Service]\nType=oneshot\nExecStart={} report run {id} --trigger timer\n",
        exec_word(&binary.to_string_lossy())
    )
}

pub fn render_timer(id: i64, schedule: &Schedule) -> String {
    format!(
        "[Unit]\nDescription=OmaFinances report {id} schedule\n\n[Timer]\nOnCalendar={}\nPersistent=true\nRandomizedDelaySec=60\nUnit={}\n\n[Install]\nWantedBy=timers.target\n",
        schedule.to_oncalendar(),
        service_file(id)
    )
}

/// An enabled report and its schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct Desired {
    pub id: i64,
    pub schedule: Schedule,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncReport {
    /// Reports whose unit files were created or changed.
    pub written: Vec<i64>,
    /// Orphaned `omafinances-report-<id>` units removed.
    pub removed: Vec<i64>,
    /// Timers that are now enabled and running.
    pub enabled: Vec<i64>,
}

fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(false);
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, content).map_err(|e| format!("write {}: {}", tmp.display(), e.kind()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {}", path.display(), e.kind()))?;
    Ok(true)
}

/// Unit ids present in `dir` (from either the `.service` or the `.timer` name).
fn unit_ids_on_disk(dir: &Path) -> Result<BTreeSet<i64>, String> {
    let mut ids = BTreeSet::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {}", dir.display(), e.kind()))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix(PREFIX) {
            if let Some(num) = rest.strip_suffix(".service").or_else(|| rest.strip_suffix(".timer")) {
                if let Ok(id) = num.parse::<i64>() {
                    ids.insert(id);
                }
            }
        }
    }
    Ok(ids)
}

/// Make the units in `dir` match `desired`. Idempotent: unchanged files are not rewritten, and `daemon-reload` runs
/// only when something was written or removed. Orphans are disabled and deleted.
pub fn sync(dir: &Path, binary: &Path, desired: &[Desired], sd: &dyn Systemd) -> Result<SyncReport, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {}", dir.display(), e.kind()))?;
    let mut report = SyncReport::default();
    let mut changed = false;
    let want: BTreeSet<i64> = desired.iter().map(|d| d.id).collect();

    for d in desired {
        let s = write_if_changed(&dir.join(service_file(d.id)), &render_service(d.id, binary))?;
        let t = write_if_changed(&dir.join(timer_file(d.id)), &render_timer(d.id, &d.schedule))?;
        if s || t {
            changed = true;
            report.written.push(d.id);
        }
    }

    for id in unit_ids_on_disk(dir)?.difference(&want) {
        let timer = timer_file(*id);
        sd.user(&["disable", "--now", &timer])?;
        for f in [timer_file(*id), service_file(*id)] {
            let _ = std::fs::remove_file(dir.join(f));
        }
        changed = true;
        report.removed.push(*id);
    }

    if changed {
        sd.user(&["daemon-reload"])?;
    }
    for d in desired {
        sd.user(&["enable", "--now", &timer_file(d.id)])?;
        report.enabled.push(d.id);
    }
    // An already-running timer keeps its old OnCalendar until restarted.
    for id in report.written.clone() {
        if desired.iter().any(|d| d.id == id) {
            sd.user(&["restart", &timer_file(id)])?;
        }
    }
    report.written.sort();
    Ok(report)
}

/// Remove one report's units (disable the timer, delete both files, reload). No-op when none exist.
pub fn remove_report(dir: &Path, id: i64, sd: &dyn Systemd) -> Result<bool, String> {
    let timer_path = dir.join(timer_file(id));
    let service_path = dir.join(service_file(id));
    if !timer_path.exists() && !service_path.exists() {
        return Ok(false);
    }
    if timer_path.exists() {
        sd.user(&["disable", "--now", &timer_file(id)])?;
    }
    let _ = std::fs::remove_file(&timer_path);
    let _ = std::fs::remove_file(&service_path);
    sd.user(&["daemon-reload"])?;
    Ok(true)
}
