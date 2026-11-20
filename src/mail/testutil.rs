//! Test-only fake HTTP server (tiny_http on 127.0.0.1, ephemeral port) that records every request.

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Recorded {
    pub path: String,
    pub auth: Option<String>,
    pub body: String,
}

pub type Handler = Box<dyn Fn(&Recorded) -> (u16, String) + Send + Sync>;

/// Serves `handler` on a background thread; returns the base URL (`http://127.0.0.1:port`) and the request log.
pub fn fake_server(handler: Handler) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind fake server");
    let base = format!("http://127.0.0.1:{}", server.server_addr().to_ip().expect("ip").port());
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let auth = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.as_str().to_string());
            let rec = Recorded {
                path: req.url().split('?').next().unwrap_or("").to_string(),
                auth,
                body,
            };
            let (status, resp) = handler(&rec);
            log2.lock().unwrap().push(rec);
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let _ = req.respond(tiny_http::Response::from_string(resp).with_status_code(status).with_header(header));
        }
    });
    (base, log)
}
