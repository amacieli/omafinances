//! Who may talk to the UI's loopback port (SEC-001 browser and DNS-rebinding vectors).
//!
//! The only legitimate client is the QML UI (`XMLHttpRequest`), which sends no `Origin` and no
//! `Sec-Fetch-*` headers and always addresses `127.0.0.1:<port>`. Browsers add `Origin` to
//! cross-site requests and `Sec-Fetch-*` to every request, and a DNS-rebinding page sends its own
//! host name. All of those are refused (403) before any route runs. Then every request must carry
//! the per-launch session token (`http::session`) as `Authorization: Bearer <token>` (401).

/// Shortest token the guard accepts. Anything shorter refuses every request.
pub const MIN_TOKEN_LEN: usize = 32;

/// Why a request was refused. 403 has no body; 401 has no body either.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Refusal {
    /// An `Origin` header, whatever its value (`null` included).
    Origin,
    /// A `Sec-Fetch-*` header: the request was made by a browser.
    FetchMetadata,
    /// No `Host`, more than one, or not exactly `127.0.0.1:<port>`.
    Host,
    /// No `Authorization: Bearer <token>`, the wrong token, or no usable token configured.
    Token,
}

impl Refusal {
    pub fn status(&self) -> u16 {
        match self {
            Refusal::Token => 401,
            _ => 403,
        }
    }
    /// Short name for the log. Never carries header values.
    pub fn label(&self) -> &'static str {
        match self {
            Refusal::Origin => "origin",
            Refusal::FetchMetadata => "fetch-metadata",
            Refusal::Host => "host",
            Refusal::Token => "token",
        }
    }
}

/// The `Host` value the UI sends to a backend on `port`.
pub fn expected_host(port: u16) -> String {
    format!("127.0.0.1:{}", port)
}

fn trimmed(v: &str) -> &str {
    v.trim_matches(|c| c == ' ' || c == '\t')
}

/// Constant-time equality for two strings (length may leak; the token length is fixed).
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Check one request's headers (name, value pairs, names compared without case) against the rules
/// above. `token` is this backend's session token. Checks run in order: Origin, Sec-Fetch-*, Host,
/// token.
pub fn check(headers: &[(&str, &str)], port: u16, token: &str) -> Result<(), Refusal> {
    let named = |n: &'static str| headers.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(n));

    if named("origin").next().is_some() {
        return Err(Refusal::Origin);
    }
    if headers
        .iter()
        .any(|(k, _)| k.as_bytes().get(..10).is_some_and(|p| p.eq_ignore_ascii_case(b"sec-fetch-")))
    {
        return Err(Refusal::FetchMetadata);
    }

    let hosts: Vec<&str> = named("host").map(|(_, v)| trimmed(v)).collect();
    if hosts.len() != 1 || hosts[0] != expected_host(port) {
        return Err(Refusal::Host);
    }

    let auth: Vec<&str> = named("authorization").map(|(_, v)| trimmed(v)).collect();
    let presented = match auth.as_slice() {
        [v] => v.strip_prefix("Bearer ").unwrap_or(""),
        _ => "",
    };
    if token.len() < MIN_TOKEN_LEN || presented.is_empty() || !ct_eq(presented, token) {
        return Err(Refusal::Token);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 27183;
    const TOK: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// The exact header set the QML XMLHttpRequest sends.
    fn ui_get() -> Vec<(&'static str, &'static str)> {
        vec![
            ("Host", "127.0.0.1:27183"),
            ("Authorization", "Bearer 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            ("Content-Type", "text/plain;charset=UTF-8"),
            ("Connection", "Keep-Alive"),
            ("Accept-Encoding", "gzip, deflate"),
            ("Accept-Language", "en-US,*"),
            ("User-Agent", "Mozilla/5.0"),
        ]
    }

    fn with(mut h: Vec<(&'static str, &'static str)>, extra: (&'static str, &'static str)) -> Vec<(&'static str, &'static str)> {
        h.push(extra);
        h
    }

    fn without(h: Vec<(&'static str, &'static str)>, name: &str) -> Vec<(&'static str, &'static str)> {
        h.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case(name)).collect()
    }

    fn replace_host(h: Vec<(&'static str, &'static str)>, host: &'static str) -> Vec<(&'static str, &'static str)> {
        h.into_iter().map(|(k, v)| if k == "Host" { (k, host) } else { (k, v) }).collect()
    }

    #[test]
    fn the_ui_gets_through() {
        assert_eq!(check(&ui_get(), PORT, TOK), Ok(()));
        // a scratch instance on its own port
        let scratch = replace_host(ui_get(), "127.0.0.1:27999");
        assert_eq!(check(&scratch, 27999, TOK), Ok(()));
    }

    #[test]
    fn trailing_whitespace_in_values_is_ignored() {
        let h = replace_host(ui_get(), "127.0.0.1:27183 ");
        assert_eq!(check(&h, PORT, TOK), Ok(()));
    }

    #[test]
    fn any_origin_is_refused() {
        for o in ["https://evil.example", "null", "http://127.0.0.1:27183", "file://", ""] {
            let h = with(ui_get(), ("Origin", o));
            assert_eq!(check(&h, PORT, TOK), Err(Refusal::Origin), "origin {:?}", o);
        }
        let h = with(ui_get(), ("oRiGiN", "https://evil.example"));
        assert_eq!(check(&h, PORT, TOK), Err(Refusal::Origin));
    }

    #[test]
    fn fetch_metadata_is_refused() {
        for (k, v) in [
            ("Sec-Fetch-Site", "cross-site"),
            ("Sec-Fetch-Mode", "no-cors"),
            ("Sec-Fetch-Dest", "image"),
            ("sec-fetch-user", "?1"),
        ] {
            let h = with(ui_get(), (k, v));
            assert_eq!(check(&h, PORT, TOK), Err(Refusal::FetchMetadata), "{}", k);
        }
    }

    #[test]
    fn host_must_be_exactly_loopback_and_port() {
        for host in [
            "attacker.example",
            "attacker.example:27183",
            "localhost:27183",
            "127.0.0.1",
            "127.0.0.1:27182",
            "[::1]:27183",
            "127.0.0.2:27183",
            "127.0.0.1:27183.evil",
            "",
        ] {
            let h = replace_host(ui_get(), host);
            assert_eq!(check(&h, PORT, TOK), Err(Refusal::Host), "host {:?}", host);
        }
        assert_eq!(check(&without(ui_get(), "Host"), PORT, TOK), Err(Refusal::Host));
        let twice = with(ui_get(), ("Host", "127.0.0.1:27183"));
        assert_eq!(check(&twice, PORT, TOK), Err(Refusal::Host));
        let evil_twice = with(ui_get(), ("Host", "evil"));
        assert_eq!(check(&evil_twice, PORT, TOK), Err(Refusal::Host));
        // right host header, wrong port for this instance
        assert_eq!(check(&ui_get(), 27999, TOK), Err(Refusal::Host));
    }

    #[test]
    fn the_session_token_is_required() {
        let no_auth = without(ui_get(), "Authorization");
        assert_eq!(check(&no_auth, PORT, TOK), Err(Refusal::Token));
        assert_eq!(Refusal::Token.status(), 401);
        assert_eq!(Refusal::Origin.status(), 403);

        let other = "f".repeat(64);
        let bad = [
            format!("Bearer {}", other),
            format!("Basic {}", TOK),
            format!("bearer {}", TOK),
            TOK.to_string(),
            "Bearer ".to_string(),
            "Bearer".to_string(),
            format!("Bearer {}x", TOK),
            format!("Bearer {}", &TOK[..63]),
        ];
        for auth in bad {
            let leaked: &'static str = Box::leak(auth.clone().into_boxed_str());
            let h = with(no_auth.clone(), ("Authorization", leaked));
            assert_eq!(check(&h, PORT, TOK), Err(Refusal::Token), "auth {:?}", auth);
        }
        // sent twice: refused even when one copy is right
        let twice = with(ui_get(), ("Authorization", "Bearer 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
        assert_eq!(check(&twice, PORT, TOK), Err(Refusal::Token));
        // a backend without a usable token refuses everything, even a matching empty bearer
        assert_eq!(check(&ui_get(), PORT, ""), Err(Refusal::Token));
        assert_eq!(check(&ui_get(), PORT, &TOK[..31]), Err(Refusal::Token));
        let empty_bearer = replace_host(no_auth.clone(), "127.0.0.1:27183");
        assert_eq!(check(&with(empty_bearer, ("Authorization", "Bearer ")), PORT, ""), Err(Refusal::Token));
        // browser and host checks come first (403), so a browser never learns about the token
        let evil = with(no_auth, ("Origin", "https://evil.example"));
        assert_eq!(check(&evil, PORT, TOK), Err(Refusal::Origin));
    }

    #[test]
    fn ct_eq_compares_exactly() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "ab"));
        assert!(!ct_eq("", "a"));
    }
}
