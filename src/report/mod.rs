//! Reports: saved query + shape + period + schedule + recipients. See `docs/PHASE6_CONTRACT.md`.
//!
//! `model` (definitions, validation), `schedule` (JSON, next runs, OnCalendar), `run` (compute, execute),
//! `render` (text/HTML/CSV), `units` (systemd user timers behind a trait), `cli` (`omafinances report …`).

pub mod cli;
pub mod mailglue;
pub mod model;
pub mod render;
pub mod run;
pub mod schedule;
pub mod units;

#[cfg(test)]
mod tests;

use crate::config::Settings;
use crate::db::Db;
use crate::mail::{MailStatus, Mailer, OutMail};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// A mailer that can be shared (tests inject one). The real Gmail mailer is built per call instead.
pub type SharedMailer = Arc<dyn Mailer + Send + Sync>;

struct Shared(SharedMailer);

impl Mailer for Shared {
    fn send(&self, mail: &OutMail) -> Result<String> {
        self.0.send(mail)
    }
    fn sender_address(&self) -> Result<String> {
        self.0.sender_address()
    }
}

/// The side effects the report routes and CLI need: a mailer and systemd. Tests replace both.
pub struct ReportHost {
    /// `None` means the real Gmail mailer, built from settings on demand.
    pub mailer: Option<SharedMailer>,
    pub systemd: Arc<dyn units::Systemd + Send + Sync>,
    /// `None` means `OMAFINANCES_SYSTEMD_DIR`, else `~/.config/systemd/user`.
    pub units_dir: Option<PathBuf>,
    /// `None` means the installed binary when present, else `current_exe()`.
    pub binary: Option<PathBuf>,
}

impl Default for ReportHost {
    fn default() -> Self {
        ReportHost { mailer: None, systemd: Arc::new(units::RealSystemd), units_dir: None, binary: None }
    }
}

impl ReportHost {
    pub fn units_dir(&self) -> PathBuf {
        self.units_dir.clone().unwrap_or_else(units::default_units_dir)
    }

    pub fn binary(&self) -> PathBuf {
        self.binary.clone().unwrap_or_else(units::default_binary)
    }

    /// The mailer for a send. Errors name the fix (for example `omafinances gmail-auth`).
    pub fn mailer(&self, settings: &Settings) -> Result<Box<dyn Mailer>> {
        match &self.mailer {
            Some(m) => Ok(Box::new(Shared(m.clone()))),
            None => mailglue::real_mailer(settings),
        }
    }

    /// The authenticated sender (the default recipient).
    pub fn sender(&self, settings: &Settings) -> Option<String> {
        self.mail_status(settings).sender
    }

    pub fn mail_status(&self, settings: &Settings) -> MailStatus {
        match &self.mailer {
            Some(m) => match m.sender_address() {
                Ok(s) => MailStatus { authenticated: true, sender: Some(s), client_configured: true },
                Err(_) => MailStatus { authenticated: false, sender: None, client_configured: true },
            },
            None => mailglue::status_for(settings).unwrap_or(MailStatus {
                authenticated: false,
                sender: None,
                client_configured: crate::mail::client_configured(settings),
            }),
        }
    }
}

/// Make the systemd user timers match the enabled reports in the database.
pub fn sync_all(db: &Db, host: &ReportHost) -> Result<units::SyncReport, String> {
    let rows = db.reports_list().map_err(|_| "could not read reports".to_string())?;
    let mut desired = Vec::new();
    for r in rows.iter().filter(|r| r.enabled) {
        let rep = model::Report::from_row(r)?;
        desired.push(units::Desired { id: rep.id, schedule: rep.schedule });
    }
    units::sync(&host.units_dir(), &host.binary(), &desired, host.systemd.as_ref())
}

/// The cache and the derived model, from the database (no UI state needed).
pub fn load_model(db: &Db) -> anyhow::Result<(crate::cache::Cache, crate::insight::Model)> {
    let cache = crate::cache::Cache::load(db)?;
    let aliases = db.payee_aliases()?;
    let rules = db.list_rules()?;
    let model = crate::insight::Model::build(&cache, &aliases, &rules);
    Ok((cache, model))
}
