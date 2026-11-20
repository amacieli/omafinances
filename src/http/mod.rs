//! The UI's loopback HTTP server (tiny_http). Binds `127.0.0.1:<port>` only.
//!
//! Every request goes through `guard::check` before routing, including unknown paths. Responses are
//! `application/json` with `Cache-Control: no-store` and never carry CORS headers. Bodies are capped at
//! `MAX_BODY`. Worker threads take requests from one shared server; the accept side never runs DB
//! work itself.
//!
//! Limitation: tiny_http 0.12 sets no per-socket read timeout. A client that stalls mid-request keeps
//! only its own connection thread busy; the workers are not affected.

pub mod guard;
pub mod session;

use crate::api::{self, AppState};
use anyhow::{anyhow, Result};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server};

/// Worker threads pulling requests from the server.
pub const WORKERS: usize = 4;
/// Largest request body accepted; bigger ones get 413 before routing.
pub const MAX_BODY: usize = 64 * 1024;

/// A running server. `shutdown` stops the workers and waits for them. Dropping the handle does not
/// stop the server.
pub struct ServerHandle {
    port: u16,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop accepting, let in-flight workers finish, and join them.
    pub fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        for w in self.workers {
            let _ = w.join();
        }
    }
}

/// Bind `127.0.0.1:<port>` and start the workers. Returns once they are running. Fails if the port
/// cannot be bound or the token is too short to be useful.
pub fn spawn(port: u16, token: String, state: Arc<AppState>) -> Result<ServerHandle> {
    if token.len() < guard::MIN_TOKEN_LEN {
        return Err(anyhow!("session token too short; refusing to serve"));
    }
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow!("cannot bind 127.0.0.1:{}: {}", port, e))?;
    let server = Arc::new(server);
    let token: Arc<str> = Arc::from(token);
    let stop = Arc::new(AtomicBool::new(false));
    log::info!("http: listening on 127.0.0.1:{}", port);

    let workers = (0..WORKERS)
        .map(|i| {
            let (server, token, state, stop) = (server.clone(), token.clone(), state.clone(), stop.clone());
            thread::Builder::new()
                .name(format!("http-worker-{}", i))
                .spawn(move || worker(&server, port, &token, &state, &stop))
                .map_err(|e| anyhow!("cannot start http worker: {}", e))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(ServerHandle { port, stop, workers })
}

/// Start the server for the app's lifetime. Returns once the workers run; they keep running.
pub fn serve(port: u16, token: String, state: Arc<AppState>) -> Result<()> {
    let _handle = spawn(port, token, state)?;
    Ok(())
}

fn worker(server: &Server, port: u16, token: &str, state: &AppState, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(req)) => handle_request(req, port, token, state),
            Ok(None) => {}
            Err(e) => {
                log::warn!("http: accept failed: {}", e.kind());
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn json_response(code: u16, body: Option<&serde_json::Value>) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = match body {
        Some(v) => serde_json::to_vec(v).unwrap_or_else(|_| b"{\"error\":\"internal\"}".to_vec()),
        None => Vec::new(),
    };
    let mut resp = Response::from_data(bytes).with_status_code(code);
    if body.is_some() {
        resp = resp.with_header(header("Content-Type", "application/json"));
    }
    resp.with_header(header("Cache-Control", "no-store"))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header is valid")
}

fn respond(req: Request, code: u16, body: Option<&serde_json::Value>) {
    if let Err(e) = req.respond(json_response(code, body)) {
        log::debug!("http: respond failed: {}", e.kind());
    }
}

fn handle_request(mut req: Request, port: u16, token: &str, state: &AppState) {
    let method = req.method().to_string();

    // Guard first, on the headers only. Nothing else is read from a refused request.
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    if let Err(refusal) = guard::check(&refs, port, token) {
        log::warn!("http: {} refused ({})", method, refusal.label());
        respond(req, refusal.status(), None);
        return;
    }

    if req.body_length().is_some_and(|n| n > MAX_BODY) {
        log::warn!("http: {} refused (body too large)", method);
        respond(req, 413, Some(&serde_json::json!({ "error": "request body too large" })));
        return;
    }
    let mut body = Vec::new();
    if req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body).is_err() {
        respond(req, 400, Some(&serde_json::json!({ "error": "unreadable request body" })));
        return;
    }
    if body.len() > MAX_BODY {
        respond(req, 413, Some(&serde_json::json!({ "error": "request body too large" })));
        return;
    }

    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let (code, value) = api::handle_body(state, &method, path, query, &body);
    log::info!("http: {} {} -> {}", method, api::route_name(path), code);
    respond(req, code, Some(&value));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn state() -> Arc<AppState> {
        Arc::new(AppState::new(Settings::default(), Err("no database in socket tests".to_string()), "test"))
    }

    /// A free loopback port: bind an ephemeral one, note it, release it.
    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    struct Reply {
        status: u16,
        head: String,
        body: String,
    }

    /// Send raw bytes and read one reply: the head, then `Content-Length` bytes of body. The server
    /// keeps connections alive (`Connection: Keep-Alive` from the UI), so reading to EOF would wait.
    fn raw(port: u16, request: &str) -> Reply {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            match s.read(&mut chunk) {
                Ok(n) if n > 0 => buf.extend_from_slice(&chunk[..n]),
                _ => break buf.len(),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end.min(buf.len())]).to_string();
        let clen: usize = head
            .lines()
            .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")))
            .and_then(|(_, v)| v.trim().parse().ok())
            .unwrap_or(0);
        while buf.len() < head_end + clen {
            match s.read(&mut chunk) {
                Ok(n) if n > 0 => buf.extend_from_slice(&chunk[..n]),
                _ => break,
            }
        }
        let body = String::from_utf8_lossy(&buf[head_end.min(buf.len())..(head_end + clen).min(buf.len())]).to_string();
        let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        Reply { status, head: head.trim_end().to_string(), body }
    }

    /// The exact header set the QML XMLHttpRequest sends, plus the path.
    fn ui_request(method: &str, path: &str, port: u16, token: &str) -> String {
        format!(
            "{m} {p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: text/plain;charset=UTF-8\r\nConnection: Keep-Alive\r\n\
             Accept-Encoding: gzip, deflate\r\nAccept-Language: en-US,*\r\nUser-Agent: Mozilla/5.0\r\n\
             Content-Length: 0\r\n\r\n",
            m = method,
            p = path,
            port = port,
            token = token
        )
    }

    fn start() -> (ServerHandle, u16) {
        let port = free_port();
        let h = spawn(port, TOKEN.to_string(), state()).unwrap();
        (h, port)
    }

    #[test]
    fn spawn_refuses_a_short_token() {
        let port = free_port();
        assert!(spawn(port, "short".to_string(), state()).is_err());
    }

    #[test]
    fn binds_loopback_only_and_fails_when_the_port_is_taken() {
        let (h, port) = start();
        assert_eq!(h.port(), port);
        // the port is now in use: a second bind must fail, not fall back to another port
        assert!(spawn(port, TOKEN.to_string(), state()).is_err());
        h.shutdown();
    }

    #[test]
    fn socket_layer_guard_and_routing() {
        let (h, port) = start();

        // no token -> 401
        let no_token = format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port);
        assert_eq!(raw(port, &no_token).status, 401);

        // wrong token -> 401
        let wrong = ui_request("GET", "/health", port, &"f".repeat(64));
        assert_eq!(raw(port, &wrong).status, 401);

        // Origin -> 403
        let origin = ui_request("GET", "/health", port, TOKEN).replace("Host:", "Origin: https://evil.example\r\nHost:");
        assert_eq!(raw(port, &origin).status, 403);

        // Sec-Fetch-Site -> 403
        let fetch = ui_request("GET", "/health", port, TOKEN).replace("Host:", "Sec-Fetch-Site: cross-site\r\nHost:");
        assert_eq!(raw(port, &fetch).status, 403);

        // wrong Host (DNS rebinding shape) -> 403
        let rebind = ui_request("GET", "/health", port, TOKEN).replace(&format!("127.0.0.1:{}", port), "attacker.example");
        assert_eq!(raw(port, &rebind).status, 403);

        // valid -> 200 JSON, no-store, no CORS
        let ok = raw(port, &ui_request("GET", "/health", port, TOKEN));
        assert_eq!(ok.status, 200);
        assert!(ok.head.to_ascii_lowercase().contains("content-type: application/json"), "{}", ok.head);
        assert!(ok.head.to_ascii_lowercase().contains("cache-control: no-store"), "{}", ok.head);
        assert_eq!(ok.body, r#"{"ok":true,"version":"test"}"#);

        // settings through the socket
        let s = raw(port, &ui_request("GET", "/settings", port, TOKEN));
        assert_eq!(s.status, 200);
        assert!(s.body.contains("scale_factor"), "{}", s.body);

        // unknown route with a valid token -> 404 (guard ran first)
        let unknown = raw(port, &ui_request("GET", "/nope", port, TOKEN));
        assert_eq!(unknown.status, 404);
        assert!(unknown.body.contains("\"error\""));

        // POST with a valid token -> 405
        assert_eq!(raw(port, &ui_request("POST", "/health", port, TOKEN)).status, 405);

        // unknown route with no token is refused by the guard, not routed
        let unknown_no_token = format!("GET /nope HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port);
        assert_eq!(raw(port, &unknown_no_token).status, 401);

        // no Access-Control-* header on any response, refused or served
        for reply in [&ok, &s, &unknown] {
            assert!(!reply.head.to_ascii_lowercase().contains("access-control-"), "{}", reply.head);
        }
        let all = [
            raw(port, &no_token),
            raw(port, &wrong),
            raw(port, &origin),
            raw(port, &fetch),
            raw(port, &rebind),
            raw(port, &ui_request("POST", "/health", port, TOKEN)),
        ];
        for r in all {
            assert!(!r.head.to_ascii_lowercase().contains("access-control-"), "{}", r.head);
        }

        h.shutdown();
    }

    #[test]
    fn new_routes_are_behind_the_guard() {
        let (h, port) = start();
        let no_token = |method: &str, path: &str| {
            format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", method, path, port)
        };
        // without a token every new route is refused by the guard, whatever the method
        for (m, p) in [("GET", "/transactions"), ("GET", "/transactions/1"), ("GET", "/accounts"), ("GET", "/categories"), ("POST", "/refresh"), ("GET", "/refresh"), ("POST", "/categories"), ("POST", "/categories/1/rename"), ("POST", "/categories/1/kind"), ("POST", "/categories/1/move"), ("POST", "/categories/1/shift"), ("POST", "/categories/1/delete"), ("POST", "/categories/1/merge"), ("POST", "/transactions/categorize"), ("POST", "/undo"), ("POST", "/redo"), ("GET", "/journal"), ("POST", "/smart/preview"), ("POST", "/smart/rows"), ("POST", "/smart/apply"), ("GET", "/rules"), ("POST", "/suggest"), ("GET", "/suggest"), ("GET", "/suggest/accuracy"), ("POST", "/suggest/accuracy"), ("GET", "/review"), ("POST", "/review"), ("GET", "/insights/recurring"), ("GET", "/insights/anomalies"), ("GET", "/insights/transfers"), ("POST", "/insights/transfers"), ("GET", "/dashboard"), ("POST", "/dashboard")] {
            assert_eq!(raw(port, &no_token(m, p)).status, 401, "{} {}", m, p);
        }
        // with a token they reach the router; this state has no database, so data routes say 503
        for (m, p, want) in [
            ("GET", "/transactions", 503),
            ("GET", "/transactions/1", 503),
            ("GET", "/accounts", 503),
            ("GET", "/categories", 503),
            ("POST", "/refresh", 503),
            ("GET", "/refresh", 405),
            ("POST", "/accounts", 405),
            ("POST", "/transactions/1", 405),
            ("GET", "/transactions/1/extra", 404),
            ("POST", "/smart/preview", 503),
            ("POST", "/smart/rows", 503),
            ("POST", "/smart/apply", 503),
            ("GET", "/rules", 503),
            ("GET", "/smart/apply", 405),
            ("POST", "/rules", 405),
            ("POST", "/suggest", 503),
            ("GET", "/suggest", 405),
            ("GET", "/suggest/accuracy", 503),
            ("POST", "/suggest/accuracy", 405),
            ("GET", "/review", 503),
            ("POST", "/review", 405),
            ("GET", "/insights/recurring", 503),
            ("GET", "/insights/anomalies", 503),
            ("GET", "/insights/transfers", 503),
            ("POST", "/insights/anomalies", 405),
            ("GET", "/dashboard", 503),
            ("POST", "/dashboard", 405),
            ("GET", "/insights", 404),
            ("GET", "/insights/other", 404),
        ] {
            assert_eq!(raw(port, &ui_request(m, p, port, TOKEN)).status, want, "{} {}", m, p);
        }
        h.shutdown();
    }

    #[test]
    fn slash_routes_are_behind_the_guard_for_every_method() {
        let (h, port) = start();
        let no_token = |method: &str, path: &str| {
            format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", method, path, port)
        };
        let routes = [
            ("GET", "/search/parse?q=p%3Aa"),
            ("GET", "/views"),
            ("POST", "/views"),
            ("DELETE", "/views/1"),
            ("DELETE", "/views"),
            ("GET", "/history"),
            ("POST", "/history"),
            ("DELETE", "/history"),
            ("PUT", "/views"),
        ];
        for (m, p) in routes {
            // no token: 401, wrong token: 401, Origin / Sec-Fetch-Site / rebinding Host: 403
            assert_eq!(raw(port, &no_token(m, p)).status, 401, "{} {}", m, p);
            assert_eq!(raw(port, &ui_request(m, p, port, &"f".repeat(64))).status, 401, "{} {}", m, p);
            let origin = ui_request(m, p, port, TOKEN).replace("Host:", "Origin: https://evil.example\r\nHost:");
            assert_eq!(raw(port, &origin).status, 403, "{} {}", m, p);
            let fetch = ui_request(m, p, port, TOKEN).replace("Host:", "Sec-Fetch-Site: cross-site\r\nHost:");
            assert_eq!(raw(port, &fetch).status, 403, "{} {}", m, p);
            let rebind = ui_request(m, p, port, TOKEN).replace(&format!("127.0.0.1:{}", port), "attacker.example");
            assert_eq!(raw(port, &rebind).status, 403, "{} {}", m, p);
        }
        // with a token they reach the router (no database and no config dir in this state)
        for (m, p, want) in [
            ("GET", "/search/parse?q=p%3Aa", 200),
            ("GET", "/views", 503),
            ("POST", "/views", 503),
            ("DELETE", "/views/1", 503),
            ("DELETE", "/views", 405),
            ("GET", "/history", 503),
            ("POST", "/history", 503),
            ("DELETE", "/history", 405),
            ("PUT", "/views", 405),
            ("GET", "/views/1", 405),
        ] {
            assert_eq!(raw(port, &ui_request(m, p, port, TOKEN)).status, want, "{} {}", m, p);
        }
        h.shutdown();
    }

    #[test]
    fn json_bodies_reach_the_handlers_over_the_socket() {
        let dir = std::env::temp_dir().join(format!("omafin-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = Arc::new(AppState::new(Settings::default(), Err("no database".to_string()), "test").with_config_dir(dir.clone()));
        let port = free_port();
        let h = spawn(port, TOKEN.to_string(), state).unwrap();
        let with_body = |method: &str, path: &str, body: &str, content_type: Option<&str>| {
            let ct = content_type.map(|c| format!("Content-Type: {}\r\n", c)).unwrap_or_default();
            format!(
                "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                method, path, port, TOKEN, ct, body.len(), body
            )
        };
        // text/plain (what QML's XMLHttpRequest sends), application/json, and no Content-Type at all
        for ct in [Some("text/plain;charset=UTF-8"), Some("application/json"), None] {
            let r = raw(port, &with_body("POST", "/history", r#"{"q":"p:amazon"}"#, ct));
            assert_eq!((r.status, r.body.as_str()), (200, r#"{"added":true,"ok":true}"#), "{:?}", ct);
        }
        let r = raw(port, &ui_request("GET", "/history", port, TOKEN));
        assert_eq!((r.status, r.body.as_str()), (200, r#"{"rows":["p:amazon"]}"#));
        // invalid JSON / not an object -> 400
        for body in ["not json", "[1]", ""] {
            assert_eq!(raw(port, &with_body("POST", "/history", body, None)).status, 400, "{:?}", body);
        }
        // a body over the limit is refused before routing, also for the new routes
        let big = "x".repeat(MAX_BODY + 1);
        assert_eq!(raw(port, &with_body("POST", "/history", &big, None)).status, 413);
        assert_eq!(raw(port, &with_body("POST", "/views", &big, None)).status, 413);
        // a body just under the limit reaches the handler (a too-long query: 400, not 413)
        let q = serde_json::json!({ "q": "a".repeat(MAX_BODY - 20) }).to_string();
        assert_eq!(raw(port, &with_body("POST", "/history", &q, None)).status, 400);
        h.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_body_is_refused_before_routing() {
        let (h, port) = start();
        let req = format!(
            "POST /health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            port,
            TOKEN,
            MAX_BODY + 1
        );
        assert_eq!(raw(port, &req).status, 413);
        h.shutdown();
    }

    #[test]
    fn many_parallel_requests_are_all_answered() {
        let (h, port) = start();
        let threads: Vec<_> = (0..16)
            .map(|_| {
                thread::spawn(move || raw(port, &ui_request("GET", "/health", port, TOKEN)).status)
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), 200);
        }
        h.shutdown();
    }

    #[test]
    fn shutdown_joins_and_frees_the_port() {
        let (h, port) = start();
        h.shutdown();
        // The workers are joined and the server is dropped; tiny_http's accept thread releases the
        // listener right after (it wakes on the drop's self-connect), so allow a short retry.
        let mut rebound = None;
        for _ in 0..100 {
            match spawn(port, TOKEN.to_string(), state()) {
                Ok(h2) => {
                    rebound = Some(h2);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        let h2 = rebound.expect("port should be free again after shutdown");
        h2.shutdown();
    }
}
