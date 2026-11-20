//! Gmail sender: `users/me/messages/send` with a refresh-token grant. Access tokens live in memory only; a 401 from
//! the API triggers exactly one refresh and retry. Recipients are checked again at send time.

use super::oauth::{self, AccessToken, ClientConfig, Endpoints, Unauthorized};
use super::{mime, recipient_allowed_in, Mailer, OutMail};
use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct GmailMailer {
    agent: ureq::Agent,
    client: ClientConfig,
    endpoints: Endpoints,
    refresh_token: Mutex<String>,
    access: Mutex<Option<(String, Instant)>>,
    sender: Mutex<Option<String>>,
    allowed: Vec<String>,
}

impl GmailMailer {
    /// `allowed` is `[reports] allowed_recipients`; the authenticated sender is always allowed on top of it.
    pub fn new(client: ClientConfig, refresh_token: String, endpoints: Endpoints, allowed: Vec<String>) -> Self {
        GmailMailer {
            agent: oauth::http_agent(),
            client,
            endpoints,
            refresh_token: Mutex::new(refresh_token),
            access: Mutex::new(None),
            sender: Mutex::new(None),
            allowed,
        }
    }

    /// A cached access token, or a fresh one from the refresh token (`force` skips the cache).
    fn access_token(&self, force: bool) -> Result<String> {
        if !force {
            let cached = self.access.lock().unwrap().clone();
            if let Some((token, until)) = cached {
                if Instant::now() < until {
                    return Ok(token);
                }
            }
        }
        let rt = self.refresh_token.lock().unwrap().clone();
        let AccessToken { token, expires_in, refresh_token } =
            oauth::refresh_access_token(&self.agent, &self.client, &self.endpoints, &rt)?;
        if let Some(rotated) = refresh_token {
            *self.refresh_token.lock().unwrap() = rotated;
        }
        let until = Instant::now() + Duration::from_secs(expires_in.saturating_sub(60));
        *self.access.lock().unwrap() = Some((token.clone(), until));
        Ok(token)
    }

    /// Run `f` with a bearer token; on 401 refresh once and retry.
    fn with_token<T>(&self, f: impl Fn(&str) -> Result<T>) -> Result<T> {
        let token = self.access_token(false)?;
        match f(&token) {
            Err(e) if e.downcast_ref::<Unauthorized>().is_some() => {
                let token = self.access_token(true)?;
                f(&token)
            }
            other => other,
        }
    }

    fn send_raw(&self, token: &str, raw_b64url: &str) -> Result<String> {
        let url = format!("{}/users/me/messages/send", self.endpoints.gmail_api_base);
        let mut resp = self
            .agent
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .send(serde_json::json!({ "raw": raw_b64url }).to_string().as_str())
            .map_err(|e| anyhow!("gmail send request failed: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if status == 401 {
            return Err(Unauthorized.into());
        }
        if !(200..300).contains(&status) {
            // Only Google's status word is surfaced; the body can echo request content.
            let reason = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["error"]["status"].as_str().map(str::to_string))
                .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(64).collect::<String>())
                .unwrap_or_default();
            bail!("gmail send failed (HTTP {status}{}{reason})", if reason.is_empty() { "" } else { ", " });
        }
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| anyhow!("gmail send returned no JSON"))?;
        v["id"].as_str().map(str::to_string).ok_or_else(|| anyhow!("gmail send returned no message id"))
    }
}

impl Mailer for GmailMailer {
    fn send(&self, mail: &OutMail) -> Result<String> {
        if mail.to.is_empty() {
            bail!("no recipients");
        }
        let sender = self.sender_address()?;
        for addr in &mail.to {
            if !recipient_allowed_in(&self.allowed, &sender, addr) {
                bail!("recipient {addr} is not in [reports] allowed_recipients; not sent");
            }
        }
        let bytes = mime::build(&sender, mail)?;
        let raw = URL_SAFE_NO_PAD.encode(&bytes);
        self.with_token(|t| self.send_raw(t, &raw))
    }

    fn sender_address(&self) -> Result<String> {
        let cached = self.sender.lock().unwrap().clone();
        if let Some(s) = cached {
            return Ok(s);
        }
        let email = self.with_token(|t| oauth::userinfo_email(&self.agent, &self.endpoints, t))?;
        *self.sender.lock().unwrap() = Some(email.clone());
        Ok(email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::testutil::{fake_server, Recorded};
    use crate::mail::{Attachment, OutMail};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const RT: &str = "RT-secret-value-123";
    const CSEC: &str = "client-secret-value-456";

    fn endpoints(base: &str) -> Endpoints {
        Endpoints {
            auth_url: format!("{base}/auth"),
            token_url: format!("{base}/token"),
            userinfo_url: format!("{base}/userinfo"),
            gmail_api_base: format!("{base}/gmail/v1"),
        }
    }

    fn mailer(base: &str, allowed: Vec<String>) -> GmailMailer {
        GmailMailer::new(ClientConfig::new("cid", CSEC), RT.into(), endpoints(base), allowed)
    }

    fn mail_to(to: &str) -> OutMail {
        OutMail {
            to: vec![to.into()],
            subject: "[OmaFinances] Weekly: last_full_week".into(),
            text: "text body".into(),
            html: "<p>html body</p>".into(),
            attachments: vec![Attachment { filename: "r.csv".into(), mime: "text/csv".into(), bytes: b"a,b\n1,2\n".to_vec() }],
        }
    }

    fn count(log: &Arc<Mutex<Vec<Recorded>>>, path: &str) -> usize {
        log.lock().unwrap().iter().filter(|r| r.path == path).count()
    }

    #[test]
    fn send_refreshes_once_on_401_and_uses_the_new_token() {
        let token_calls = Arc::new(AtomicUsize::new(0));
        let tc = token_calls.clone();
        let (base, log) = fake_server(Box::new(move |r| match r.path.as_str() {
            "/token" => {
                let n = tc.fetch_add(1, Ordering::SeqCst) + 1;
                assert!(r.body.contains("grant_type=refresh_token"));
                (200, format!(r#"{{"access_token":"AT-{n}","expires_in":3600}}"#))
            }
            "/userinfo" => (200, r#"{"email":"me@example.com"}"#.into()),
            "/gmail/v1/users/me/messages/send" => {
                if r.auth.as_deref() == Some("Bearer AT-1") {
                    (401, r#"{"error":{"code":401,"status":"UNAUTHENTICATED"}}"#.into())
                } else {
                    (200, r#"{"id":"msg-42","threadId":"t"}"#.into())
                }
            }
            _ => (404, "{}".into()),
        }));
        let m = mailer(&base, vec!["ops@example.com".into()]);
        let id = m.send(&mail_to("ops@example.com")).unwrap();
        assert_eq!(id, "msg-42");
        assert_eq!(token_calls.load(Ordering::SeqCst), 2, "one refresh at start, one after the 401");
        assert_eq!(count(&log, "/gmail/v1/users/me/messages/send"), 2);
        let sends: Vec<String> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path.ends_with("/send"))
            .map(|r| r.auth.clone().unwrap())
            .collect();
        assert_eq!(sends, vec!["Bearer AT-1".to_string(), "Bearer AT-2".to_string()]);
        // The posted message is base64url MIME with the expected headers.
        let body: serde_json::Value = serde_json::from_str(
            &log.lock().unwrap().iter().find(|r| r.path.ends_with("/send")).unwrap().body.clone(),
        )
        .unwrap();
        let raw = URL_SAFE_NO_PAD.decode(body["raw"].as_str().unwrap()).unwrap();
        let raw = String::from_utf8(raw).unwrap();
        assert!(raw.starts_with("From: me@example.com\r\nTo: ops@example.com\r\n"));
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("filename*=UTF-8''r.csv"));
    }

    #[test]
    fn revoked_grant_says_to_reauthorize_and_never_contains_a_secret() {
        let (base, log) = fake_server(Box::new(|r| match r.path.as_str() {
            "/token" => (400, format!(r#"{{"error":"invalid_grant","error_description":"echo {RT} {CSEC}"}}"#)),
            _ => (500, "should not be reached".into()),
        }));
        let m = mailer(&base, vec![]);
        let err = m.send(&mail_to("me@example.com")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("omafinances gmail-auth"), "{msg}");
        assert!(!msg.contains(RT) && !msg.contains(CSEC) && !msg.contains("echo"), "{msg}");
        assert_eq!(count(&log, "/userinfo"), 0);
    }

    #[test]
    fn token_endpoint_garbage_is_not_echoed() {
        let (base, _log) = fake_server(Box::new(|_| (500, format!("upstream said {RT} {CSEC}"))));
        let m = mailer(&base, vec![]);
        let msg = format!("{:#}", m.sender_address().unwrap_err());
        assert!(!msg.contains(RT) && !msg.contains(CSEC), "{msg}");
        assert!(msg.contains("500"));
    }

    #[test]
    fn disallowed_recipient_is_refused_before_any_send_request() {
        let (base, log) = fake_server(Box::new(|r| match r.path.as_str() {
            "/token" => (200, r#"{"access_token":"AT","expires_in":3600}"#.into()),
            "/userinfo" => (200, r#"{"email":"me@example.com"}"#.into()),
            _ => (200, r#"{"id":"should-not-happen"}"#.into()),
        }));
        let m = mailer(&base, vec!["accountant@firm.example".into()]);
        let err = m.send(&mail_to("evil@example.org")).unwrap_err();
        assert!(format!("{err:#}").contains("not in [reports] allowed_recipients"));
        assert_eq!(count(&log, "/gmail/v1/users/me/messages/send"), 0);

        // Allowlisted (case-insensitive) and the sender itself both go through.
        let ok = m.send(&mail_to("ACCOUNTANT@firm.example")).unwrap();
        assert_eq!(ok, "should-not-happen");
        assert!(m.send(&mail_to("me@example.com")).is_ok());
        assert_eq!(count(&log, "/gmail/v1/users/me/messages/send"), 2);
    }

    #[test]
    fn sender_address_is_cached_and_a_bad_address_in_the_list_is_ignored() {
        let (base, log) = fake_server(Box::new(|r| match r.path.as_str() {
            "/token" => (200, r#"{"access_token":"AT","expires_in":3600}"#.into()),
            "/userinfo" => (200, r#"{"email":"me@example.com"}"#.into()),
            _ => (404, "{}".into()),
        }));
        let m = mailer(&base, vec!["not an address, evil@example.org".into()]);
        assert_eq!(m.sender_address().unwrap(), "me@example.com");
        assert_eq!(m.sender_address().unwrap(), "me@example.com");
        assert_eq!(count(&log, "/userinfo"), 1);
        assert!(!recipient_allowed_in(&["not an address, evil@example.org".into()], "me@example.com", "evil@example.org"));
    }
}
