//! Outbound mail for scheduled reports. Own Gmail token (never omalook's, never Hermes's), send-only scope.
//! `Mailer` is the seam: reports talk to the trait, tests use a fake, `gmail::GmailMailer` is the real one.

pub mod cli;
pub mod gmail;
pub mod mime;
pub mod oauth;
#[cfg(test)]
pub(crate) mod testutil;

use crate::config::{self, Settings};
use crate::secrets;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutMail {
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub attachments: Vec<Attachment>,
}

pub trait Mailer {
    /// Send one message; returns the provider's message id.
    fn send(&self, mail: &OutMail) -> Result<String>;
    /// The authenticated sender address (the default and only allowed recipient until the allowlist grows).
    fn sender_address(&self) -> Result<String>;
}

/// Mail state for the UI / `gmail-status`. Never contains a token.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MailStatus {
    pub authenticated: bool,
    pub sender: Option<String>,
    pub client_configured: bool,
}

/// `[mail] client_secret_file` with a leading `~/` expanded; `None` when unset.
pub fn client_secret_path(settings: &Settings) -> Option<PathBuf> {
    let raw = settings.mail.client_secret_file.trim();
    if raw.is_empty() {
        return None;
    }
    Some(match raw.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(raw)),
        None => PathBuf::from(raw),
    })
}

/// Whether the configured client JSON exists and parses (the secret itself is never returned).
pub fn client_configured(settings: &Settings) -> bool {
    client_secret_path(settings).map_or(false, |p| oauth::load_client(&p).is_ok())
}

/// Status using the settings file (`~/.config/omafinances/settings.toml`).
pub fn status() -> Result<MailStatus> {
    let settings = config::load_or_init(&config::config_dir()?)?;
    status_for(&settings)
}

/// Authenticated = a refresh token is stored AND it still yields the sender address. A revoked grant reports
/// `authenticated: false`; transport or keyring failures are errors.
pub fn status_for(settings: &Settings) -> Result<MailStatus> {
    let client_configured = client_configured(settings);
    let unauthenticated = MailStatus { authenticated: false, sender: None, client_configured };
    let Some(token) = secrets::gmail_refresh_token()? else {
        return Ok(unauthenticated);
    };
    let Some(client) = load_client_for(settings).ok() else {
        return Ok(unauthenticated);
    };
    let mailer = gmail::GmailMailer::new(
        client,
        token,
        oauth::Endpoints::from_env()?,
        settings.reports.allowed_recipients.clone(),
    );
    match mailer.sender_address() {
        Ok(sender) => Ok(MailStatus { authenticated: true, sender: Some(sender), client_configured }),
        Err(e) if e.downcast_ref::<oauth::AuthError>().is_some() => Ok(unauthenticated),
        Err(e) => Err(e),
    }
}

fn load_client_for(settings: &Settings) -> Result<oauth::ClientConfig> {
    let path = client_secret_path(settings)
        .context("[mail] client_secret_file is not set in settings.toml (path to the Google OAuth client JSON)")?;
    oauth::load_client(&path)
}

/// The Gmail sender for reports. Fails (without network) when no token is stored or the client is not configured.
pub fn real_mailer(settings: &Settings) -> Result<Box<dyn Mailer>> {
    let client = load_client_for(settings)?;
    let Some(token) = secrets::gmail_refresh_token()? else {
        bail!("Gmail is not authorized: run `omafinances gmail-auth`");
    };
    Ok(Box::new(gmail::GmailMailer::new(
        client,
        token,
        oauth::Endpoints::from_env()?,
        settings.reports.allowed_recipients.clone(),
    )))
}

/// Whether reports may be sent to `addr`: the sender, or an entry of `[reports] allowed_recipients`. Case-insensitive,
/// exact match, and only plain mailboxes (no whitespace, commas, angle brackets or line breaks).
pub fn recipient_allowed(settings: &Settings, sender: &str, addr: &str) -> bool {
    recipient_allowed_in(&settings.reports.allowed_recipients, sender, addr)
}

pub(crate) fn recipient_allowed_in(allowed: &[String], sender: &str, addr: &str) -> bool {
    if !is_plain_address(addr) {
        return false;
    }
    std::iter::once(sender)
        .chain(allowed.iter().map(String::as_str))
        .any(|candidate| is_plain_address(candidate) && candidate.eq_ignore_ascii_case(addr))
}

/// A conservative ASCII mailbox check: one `@`, local part of `[A-Za-z0-9._%+-]`, dotted domain of LDH labels.
pub fn is_plain_address(s: &str) -> bool {
    if s.is_empty() || s.len() > 254 {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-'));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    local_ok && domain_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipient_allowlist_defaults_to_sender_and_is_exact_and_case_insensitive() {
        let mut s = Settings::default();
        let sender = "me@example.com";
        assert!(recipient_allowed(&s, sender, "me@example.com"));
        assert!(recipient_allowed(&s, sender, "ME@Example.COM"));
        assert!(!recipient_allowed(&s, sender, "other@example.com"));
        assert!(!recipient_allowed(&s, sender, "me@example.com.evil.net"));
        assert!(!recipient_allowed(&s, sender, "x@me@example.com"));
        s.reports.allowed_recipients = vec!["Accountant@Firm.example".into()];
        assert!(recipient_allowed(&s, sender, "accountant@firm.example"));
        assert!(recipient_allowed(&s, sender, "me@example.com"));
        assert!(!recipient_allowed(&s, sender, "someone@firm.example"));
    }

    #[test]
    fn recipients_with_separators_or_brackets_are_refused() {
        let s = Settings::default();
        let sender = "me@example.com";
        for bad in [
            "me@example.com, evil@example.org",
            "me@example.com,evil@example.org",
            "<me@example.com>",
            "Me <me@example.com>",
            "me@example.com\r\nBcc: evil@example.org",
            "me@example.com\n",
            " me@example.com",
            "me@example.com\t",
            "\"me\"@example.com",
            "me@localhost",
            "",
        ] {
            assert!(!recipient_allowed(&s, sender, bad), "must refuse {bad:?}");
        }
    }

    #[test]
    fn mail_and_reports_settings_parse_from_toml_without_secrets() {
        let t = "[mail]\nclient_secret_file = \"~/.config/omafinances/client.json\"\n[reports]\nallowed_recipients = [\"a@example.com\"]\n";
        let s: Settings = toml::from_str(t).unwrap();
        assert_eq!(s.mail.client_secret_file, "~/.config/omafinances/client.json");
        assert_eq!(s.reports.allowed_recipients, vec!["a@example.com".to_string()]);
        assert!(client_secret_path(&s).is_some());
        assert!(client_secret_path(&Settings::default()).is_none());
    }
}
