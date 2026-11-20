//! OAuth 2.0 for an installed app: authorization-code flow over a loopback redirect with PKCE (RFC 7636).
//! The client secret, tokens and authorization codes are never printed and never put into error text.

use super::is_plain_address;
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};
use url::Url;

/// Send-only Gmail plus the email address (to show who is authorized).
pub const SCOPES: &str = "https://www.googleapis.com/auth/gmail.send https://www.googleapis.com/auth/userinfo.email";
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const DEFAULT_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DEFAULT_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const DEFAULT_GMAIL_API: &str = "https://gmail.googleapis.com/gmail/v1";

/// Token/consent refused or revoked: the user must run `omafinances gmail-auth` again.
#[derive(Debug)]
pub struct AuthError(pub String);

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AuthError {}

/// HTTP 401 from a Google API call with a bearer token (the access token expired; a refresh may help).
#[derive(Debug)]
pub struct Unauthorized;

impl fmt::Display for Unauthorized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HTTP 401 unauthorized")
    }
}
impl std::error::Error for Unauthorized {}

/// Endpoints. Production values unless overridden by env (`OMAFINANCES_GMAIL_API_BASE`, `OMAFINANCES_OAUTH_AUTH_URL`,
/// `OMAFINANCES_OAUTH_TOKEN_URL`, `OMAFINANCES_USERINFO_URL`). Overrides must be https, or http on loopback (tests).
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoints {
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub gmail_api_base: String,
}

impl Endpoints {
    pub fn production() -> Self {
        Endpoints {
            auth_url: DEFAULT_AUTH_URL.into(),
            token_url: DEFAULT_TOKEN_URL.into(),
            userinfo_url: DEFAULT_USERINFO_URL.into(),
            gmail_api_base: DEFAULT_GMAIL_API.into(),
        }
    }

    pub fn from_env() -> Result<Self> {
        fn pick(key: &str, default: &str) -> Result<String> {
            match std::env::var(key) {
                Ok(v) if !v.trim().is_empty() => {
                    check_endpoint(key, v.trim())?;
                    Ok(v.trim().trim_end_matches('/').to_string())
                }
                _ => Ok(default.to_string()),
            }
        }
        Ok(Endpoints {
            auth_url: pick("OMAFINANCES_OAUTH_AUTH_URL", DEFAULT_AUTH_URL)?,
            token_url: pick("OMAFINANCES_OAUTH_TOKEN_URL", DEFAULT_TOKEN_URL)?,
            userinfo_url: pick("OMAFINANCES_USERINFO_URL", DEFAULT_USERINFO_URL)?,
            gmail_api_base: pick("OMAFINANCES_GMAIL_API_BASE", DEFAULT_GMAIL_API)?,
        })
    }
}

fn check_endpoint(key: &str, v: &str) -> Result<()> {
    let url = Url::parse(v).map_err(|_| anyhow!("{key} is not a valid URL"))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"));
    let ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    if !ok {
        bail!("{key} must be https (plain http is only accepted on loopback, for tests)");
    }
    Ok(())
}

/// The OAuth client from Google's installed-app JSON. Debug output is redacted.
#[derive(Clone)]
pub struct ClientConfig {
    client_id: String,
    client_secret: String,
}

impl fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientConfig").field("client_id", &self.client_id).field("client_secret", &"<redacted>").finish()
    }
}

impl ClientConfig {
    pub fn new(client_id: &str, client_secret: &str) -> Self {
        ClientConfig { client_id: client_id.into(), client_secret: client_secret.into() }
    }
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
}

#[derive(Deserialize)]
struct ClientFile {
    installed: Option<ClientSection>,
}

#[derive(Deserialize)]
struct ClientSection {
    client_id: String,
    client_secret: String,
}

/// Read the client JSON at `path`. Errors never include the file's contents.
pub fn load_client(path: &Path) -> Result<ClientConfig> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read client secret file {}", path.display()))?;
    parse_client(&text)
}

pub fn parse_client(text: &str) -> Result<ClientConfig> {
    let file: ClientFile = serde_json::from_str(text)
        .map_err(|_| anyhow!("client secret file is not a Google OAuth client JSON (expected an \"installed\" client)"))?;
    let s = file.installed.context("client secret file has no \"installed\" client (use a Desktop app OAuth client)")?;
    if s.client_id.trim().is_empty() || s.client_secret.trim().is_empty() {
        bail!("client secret file is missing client_id or client_secret");
    }
    Ok(ClientConfig { client_id: s.client_id, client_secret: s.client_secret })
}

/// PKCE pair (RFC 7636, S256).
#[derive(Clone, PartialEq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pkce").field("verifier", &"<redacted>").field("challenge", &self.challenge).finish()
    }
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn new_pkce() -> Pkce {
    let verifier = random_urlsafe(32); // 43 chars, within the 43..=128 range
    let challenge = pkce_challenge(&verifier);
    Pkce { verifier, challenge }
}

fn random_urlsafe(n: usize) -> String {
    let mut b = vec![0u8; n];
    OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(&b)
}

/// Result of a consent: the long-lived refresh token (store it, nothing else) and a first access token.
#[derive(Clone)]
pub struct Grant {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_in: u64,
}

impl fmt::Debug for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grant")
            .field("refresh_token", &"<redacted>")
            .field("access_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// A refreshed access token; `refresh_token` is set only if Google rotated it.
#[derive(Clone)]
pub struct AccessToken {
    pub token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Agent for all calls: HTTP status is returned as data (not an error) so 401/400 bodies can be classified.
pub fn http_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    ureq::Agent::new_with_config(config)
}

/// Full consent: listens on 127.0.0.1 (ephemeral port), hands the consent URL to `on_url`, waits for the redirect
/// (`timeout`), checks `state`, and exchanges the code with the PKCE verifier.
pub fn authorize(
    client: &ClientConfig,
    ep: &Endpoints,
    timeout: Duration,
    on_url: &mut dyn FnMut(&str),
) -> Result<Grant> {
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| anyhow!("cannot open loopback listener: {e}"))?;
    let port = server.server_addr().to_ip().ok_or_else(|| anyhow!("loopback listener has no IP address"))?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let pkce = new_pkce();
    let state = random_urlsafe(16);
    let mut url = Url::parse(&ep.auth_url).context("auth URL")?;
    url.query_pairs_mut()
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state);
    on_url(url.as_str());
    let code = wait_for_code(&server, &state, timeout)?;
    drop(server);
    exchange_code(&http_agent(), client, ep, &code, &pkce.verifier, &redirect)
}

fn wait_for_code(server: &tiny_http::Server, state: &str, timeout: Duration) -> Result<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for the Google redirect (5 minutes); run gmail-auth again");
        }
        let req = match server.recv_timeout(remaining).context("loopback listener failed")? {
            Some(r) => r,
            None => continue,
        };
        let target = Url::parse(&format!("http://127.0.0.1{}", req.url())).ok();
        let path = target.as_ref().map(|u| u.path().to_string()).unwrap_or_default();
        if path != "/callback" {
            respond(req, 404, "Not found.");
            continue;
        }
        let params: HashMap<String, String> =
            target.map(|u| u.query_pairs().into_owned().collect()).unwrap_or_default();
        if let Some(err) = params.get("error") {
            respond(req, 400, "Access was not granted. You can close this tab.");
            bail!("Google did not grant access ({})", sanitize_code(err));
        }
        if params.get("state").map(String::as_str) != Some(state) {
            // Not our redirect (stray request or a replayed one): refuse it and keep waiting for the real one.
            respond(req, 400, "Unexpected request. You can close this tab.");
            continue;
        }
        let Some(code) = params.get("code").cloned() else {
            respond(req, 400, "No authorization code was returned. You can close this tab.");
            bail!("the Google redirect carried no authorization code");
        };
        respond(req, 200, "OmaFinances is authorized. You can close this tab.");
        return Ok(code);
    }
}

fn respond(req: tiny_http::Request, status: u16, body: &str) {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..])
        .expect("static header");
    let _ = req.respond(tiny_http::Response::from_string(body).with_status_code(status).with_header(header));
}

/// Short, printable token for error text (OAuth error codes are lowercase words).
fn sanitize_code(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(64).collect()
}

#[derive(Deserialize, Default)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
}

fn post_token(agent: &ureq::Agent, url: &str, fields: &[(&str, &str)]) -> Result<TokenResponse> {
    let mut resp = agent
        .post(url)
        .send_form(fields.to_vec())
        .map_err(|e| anyhow!("token endpoint unreachable: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|_| anyhow!("token endpoint response unreadable (HTTP {status})"))?;
    let parsed: TokenResponse = serde_json::from_str(&text).unwrap_or_default();
    if let Some(code) = parsed.error.as_deref() {
        if code == "invalid_grant" || code == "unauthorized_client" || code == "invalid_client" {
            return Err(AuthError(
                "Gmail authorization was revoked or has expired. Run: omafinances gmail-auth".into(),
            )
            .into());
        }
        bail!("token endpoint refused the request (HTTP {status}, {})", sanitize_code(code));
    }
    if !(200..300).contains(&status) {
        bail!("token endpoint refused the request (HTTP {status})");
    }
    Ok(parsed)
}

fn exchange_code(
    agent: &ureq::Agent,
    client: &ClientConfig,
    ep: &Endpoints,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<Grant> {
    let fields = [
        ("code", code),
        ("client_id", client.client_id.as_str()),
        ("client_secret", client.client_secret.as_str()),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect),
    ];
    let t = post_token(agent, &ep.token_url, &fields)?;
    let refresh_token = t.refresh_token.filter(|s| !s.is_empty()).ok_or_else(|| {
        anyhow!("Google returned no refresh token; remove OmaFinances from your Google account's third-party access and run gmail-auth again")
    })?;
    let access_token = t.access_token.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("token endpoint returned no access token"))?;
    Ok(Grant { refresh_token, access_token, expires_in: t.expires_in.unwrap_or(3600) })
}

/// Trade a refresh token for a new access token.
pub fn refresh_access_token(
    agent: &ureq::Agent,
    client: &ClientConfig,
    ep: &Endpoints,
    refresh_token: &str,
) -> Result<AccessToken> {
    let fields = [
        ("client_id", client.client_id.as_str()),
        ("client_secret", client.client_secret.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    let t = post_token(agent, &ep.token_url, &fields)?;
    let token = t.access_token.filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("token endpoint returned no access token"))?;
    Ok(AccessToken {
        token,
        expires_in: t.expires_in.unwrap_or(3600),
        refresh_token: t.refresh_token.filter(|s| !s.is_empty()),
    })
}

/// The address of the account behind `access_token` (needs `userinfo.email`).
pub fn userinfo_email(agent: &ureq::Agent, ep: &Endpoints, access_token: &str) -> Result<String> {
    let mut resp = agent
        .get(&ep.userinfo_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .call()
        .map_err(|e| anyhow!("userinfo endpoint unreachable: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        return Err(Unauthorized.into());
    }
    if !(200..300).contains(&status) {
        bail!("userinfo endpoint refused the request (HTTP {status})");
    }
    let text = resp.body_mut().read_to_string().map_err(|_| anyhow!("userinfo response unreadable"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| anyhow!("userinfo response is not JSON"))?;
    let email = v.get("email").and_then(|e| e.as_str()).unwrap_or("");
    if !is_plain_address(email) {
        bail!("userinfo response carried no usable email address");
    }
    Ok(email.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::testutil::fake_server;
    use std::sync::{Arc, Mutex};

    #[test]
    fn pkce_matches_rfc7636_appendix_b_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(pkce_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let p = new_pkce();
        assert!((43..=128).contains(&p.verifier.len()));
        assert_eq!(pkce_challenge(&p.verifier), p.challenge);
        assert!(!p.verifier.contains(['+', '/', '=']));
    }

    #[test]
    fn client_json_is_parsed_and_its_errors_never_echo_the_secret() {
        let ok = r#"{"installed":{"client_id":"id-123.apps","client_secret":"SECRET-XYZ","redirect_uris":["http://localhost"]}}"#;
        let c = parse_client(ok).unwrap();
        assert_eq!(c.client_id(), "id-123.apps");
        assert!(!format!("{c:?}").contains("SECRET-XYZ"), "Debug must redact");

        let web = r#"{"web":{"client_id":"x","client_secret":"SECRET-WEB"}}"#;
        let err = format!("{:#}", parse_client(web).unwrap_err());
        assert!(!err.contains("SECRET-WEB"));
        let broken = r#"{"installed":{"client_id":"x","client_secret":"SECRET-BROKEN""#;
        let err = format!("{:#}", parse_client(broken).unwrap_err());
        assert!(!err.contains("SECRET-BROKEN"));
        let empty = r#"{"installed":{"client_id":"x","client_secret":"  "}}"#;
        assert!(parse_client(empty).is_err());
    }

    #[test]
    fn endpoint_overrides_must_be_https_or_loopback() {
        assert!(check_endpoint("K", "https://oauth2.googleapis.com/token").is_ok());
        assert!(check_endpoint("K", "http://127.0.0.1:9999/token").is_ok());
        assert!(check_endpoint("K", "http://localhost:9999/token").is_ok());
        assert!(check_endpoint("K", "http://evil.example/token").is_err());
        assert!(check_endpoint("K", "http://127.0.0.1.evil.example/token").is_err());
        assert!(check_endpoint("K", "not a url").is_err());
    }

    fn test_endpoints(base: &str) -> Endpoints {
        Endpoints {
            auth_url: format!("{base}/auth"),
            token_url: format!("{base}/token"),
            userinfo_url: format!("{base}/userinfo"),
            gmail_api_base: format!("{base}/gmail/v1"),
        }
    }

    /// Simulates the browser: reads the consent URL, then hits the loopback redirect with `state` (and a code).
    fn browser_redirect(consent_url: &str, extra: &str, use_state: bool) -> std::thread::JoinHandle<u16> {
        let u = Url::parse(consent_url).unwrap();
        let redirect = u.query_pairs().find(|(k, _)| k == "redirect_uri").unwrap().1.into_owned();
        let state = u.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
        let target = if use_state {
            format!("{redirect}?state={state}{extra}")
        } else {
            format!("{redirect}?state=WRONG{extra}")
        };
        std::thread::spawn(move || {
            let resp = http_agent().get(&target).call();
            match resp {
                Ok(r) => r.status().as_u16(),
                Err(_) => 0,
            }
        })
    }

    #[test]
    fn full_loopback_flow_binds_pkce_and_returns_refresh_token() {
        let seen_token_body = Arc::new(Mutex::new(String::new()));
        let s2 = seen_token_body.clone();
        let (base, log) = fake_server(Box::new(move |r| match r.path.as_str() {
            "/token" => {
                *s2.lock().unwrap() = r.body.clone();
                (200, r#"{"access_token":"AT-flow","expires_in":3599,"refresh_token":"RT-flow","token_type":"Bearer"}"#.into())
            }
            _ => (404, "{}".into()),
        }));
        let ep = test_endpoints(&base);
        let client = ClientConfig::new("cid", "csecret");
        let mut consent = String::new();
        let grant = authorize(&client, &ep, Duration::from_secs(20), &mut |url| {
            consent = url.to_string();
            // A stray request with the wrong state must be refused without ending the flow.
            browser_redirect(url, "&code=stray", false).join().unwrap();
            browser_redirect(url, "&code=AUTHCODE-1", true);
        })
        .unwrap();
        assert_eq!(grant.refresh_token, "RT-flow");
        assert_eq!(grant.access_token, "AT-flow");
        assert!(!format!("{grant:?}").contains("RT-flow"));

        let u = Url::parse(&consent).unwrap();
        let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["scope"], SCOPES);
        // The token exchange must present the verifier whose S256 hash is the challenge that was sent.
        let body: HashMap<String, String> = url::form_urlencoded::parse(seen_token_body.lock().unwrap().as_bytes())
            .into_owned()
            .collect();
        assert_eq!(body["grant_type"], "authorization_code");
        assert_eq!(body["code"], "AUTHCODE-1");
        assert_eq!(pkce_challenge(&body["code_verifier"]), q["code_challenge"]);
        assert_eq!(body["client_secret"], "csecret");
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn consent_error_and_timeout_are_reported_without_secrets() {
        let (base, _log) = fake_server(Box::new(|_| (500, "{}".into())));
        let ep = test_endpoints(&base);
        let client = ClientConfig::new("cid", "SECRET-NEVER-PRINTED");
        let err = authorize(&client, &ep, Duration::from_secs(20), &mut |url| {
            browser_redirect(url, "&error=access_denied", true).join().unwrap();
        })
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("access_denied"));
        assert!(!msg.contains("SECRET-NEVER-PRINTED"));

        let err = authorize(&client, &ep, Duration::from_millis(300), &mut |_| {}).unwrap_err();
        assert!(format!("{err:#}").contains("timed out"));
    }

    #[test]
    fn refresh_rejects_revoked_grant_with_guidance_and_no_token_text() {
        let (base, _log) = fake_server(Box::new(|_| {
            (400, r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked. RT-LEAK-CHECK"}"#.into())
        }));
        let ep = test_endpoints(&base);
        let client = ClientConfig::new("cid", "CSEC-LEAK-CHECK");
        let err = refresh_access_token(&http_agent(), &client, &ep, "RT-LEAK-CHECK").unwrap_err();
        assert!(err.downcast_ref::<AuthError>().is_some());
        let msg = format!("{err:#}");
        assert!(msg.contains("gmail-auth"));
        assert!(!msg.contains("LEAK-CHECK"));
    }

    #[test]
    fn userinfo_401_is_classified_and_bad_email_is_refused() {
        let (base, _log) = fake_server(Box::new(|r| {
            if r.auth.as_deref() == Some("Bearer GOOD") {
                (200, r#"{"email":"me@example.com"}"#.into())
            } else if r.auth.as_deref() == Some("Bearer BAD") {
                (200, r#"{"email":"me@example.com, evil@example.org"}"#.into())
            } else {
                (401, "{}".into())
            }
        }));
        let ep = test_endpoints(&base);
        assert_eq!(userinfo_email(&http_agent(), &ep, "GOOD").unwrap(), "me@example.com");
        let e = userinfo_email(&http_agent(), &ep, "NOPE").unwrap_err();
        assert!(e.downcast_ref::<Unauthorized>().is_some());
        assert!(userinfo_email(&http_agent(), &ep, "BAD").is_err());
    }
}
