//! Thin shim over the mail lane's landed surface (`crate::mail`): settings reads and the real mailer. Only this file
//! changes if the mail or config surface moves. `exclude_transfers` has no settings key yet (contract default: true).

use crate::config::Settings;
use crate::mail::{self, Mailer, MailStatus};
use anyhow::Result;

/// Shown as the 409 text and as the stored run detail when no Gmail token is available.
pub const NOT_AUTHENTICATED: &str = "Gmail is not authorized: run `omafinances gmail-auth`";

/// `[reports] allowed_recipients`, in addition to the authenticated sender (which is always allowed).
pub fn allowlist(settings: &Settings) -> Vec<String> {
    settings.reports.allowed_recipients.clone()
}

/// Transfers between own accounts are excluded from report totals unless this is false. No settings key yet.
pub fn exclude_transfers(_settings: &Settings) -> bool {
    true
}

pub fn real_mailer(settings: &Settings) -> Result<Box<dyn Mailer>> {
    mail::real_mailer(settings)
}

pub fn status_for(settings: &Settings) -> Result<MailStatus> {
    mail::status_for(settings)
}
