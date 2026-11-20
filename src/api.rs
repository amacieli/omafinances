//! The UI's JSON API: a pure router, so it can be tested without sockets.
//!
//! GET only. Paths are matched exactly (no prefix or substring matching). Unknown paths are 404,
//! known paths with another method are 405. Bodies are `{"error": "..."}` on failure. Errors never
//! carry payee, amount or token data.

use crate::config::Settings;
use crate::db::Db;
use serde_json::{json, Value};

/// Everything the routes read. Built once at startup; shared (read-only) by the workers.
pub struct AppState {
    pub settings: Settings,
    /// The database, or why it could not be opened (shown by `/status`).
    pub db: Result<Db, String>,
    pub version: &'static str,
}

pub const DEFAULT_LIMIT: i64 = 100;
pub const MAX_LIMIT: i64 = 500;

/// Route name for logs. Never includes the query string.
pub fn route_name(path: &str) -> &'static str {
    match path {
        "/health" => "health",
        "/status" => "status",
        "/transactions" => "transactions",
        "/settings" => "settings",
        _ => "unknown",
    }
}

fn error(code: u16, message: &str) -> (u16, Value) {
    (code, json!({ "error": message }))
}

/// Value of the first `key=value` pair in a query string. No percent-decoding: the API only takes
/// numbers.
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// Route one request. Returns the status code and the JSON body.
pub fn handle(state: &AppState, method: &str, path: &str, query: &str) -> (u16, Value) {
    let known = matches!(path, "/health" | "/status" | "/transactions" | "/settings");
    if !known {
        return error(404, "not found");
    }
    if method != "GET" {
        return error(405, "method not allowed");
    }
    match path {
        "/health" => (200, json!({ "ok": true, "version": state.version })),
        "/status" => (200, status(state)),
        "/transactions" => transactions(state, query),
        "/settings" => (200, json!({ "scale_factor": state.settings.ui.scale_factor })),
        _ => error(404, "not found"),
    }
}

fn status(state: &AppState) -> Value {
    let mcp = json!({ "enabled": state.settings.mcp.enabled });
    let version = state.version;
    match &state.db {
        Err(message) => json!({
            "version": version,
            "schema_version": null,
            "db": {
                "state": "error", "message": message,
                "transactions": 0, "accounts": 0, "categories": 0,
                "last_txn_date": null, "last_sync": null
            },
            "mcp": mcp,
        }),
        Ok(db) => match db.status() {
            Ok(s) => json!({
                "version": version,
                "schema_version": s.schema_version,
                "db": {
                    "state": "ok", "message": null,
                    "transactions": s.transactions, "accounts": s.accounts, "categories": s.categories,
                    "last_txn_date": s.last_txn_date, "last_sync": s.last_sync
                },
                "mcp": mcp,
            }),
            Err(e) => {
                // The detail is logged, not returned: keep the UI's view generic.
                log::error!("status: database query failed: {:#}", e);
                json!({
                    "version": version,
                    "schema_version": null,
                    "db": {
                        "state": "error", "message": "database query failed",
                        "transactions": 0, "accounts": 0, "categories": 0,
                        "last_txn_date": null, "last_sync": null
                    },
                    "mcp": mcp,
                })
            }
        },
    }
}

fn transactions(state: &AppState, query: &str) -> (u16, Value) {
    let db = match &state.db {
        Ok(db) => db,
        Err(_) => return error(503, "database unavailable"),
    };
    let limit = query_param(query, "limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);
    let offset = query_param(query, "offset")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    match db.transactions(limit, offset) {
        Ok((total, rows)) => (200, json!({ "total": total, "rows": rows })),
        Err(e) => {
            log::error!("transactions: database query failed: {:#}", e);
            error(500, "database query failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_err() -> AppState {
        AppState {
            settings: Settings::default(),
            db: Err("cannot open /x/finances.db: file is not a database".to_string()),
            version: "9.9.9-test",
        }
    }

    #[test]
    fn health_reports_version() {
        let (code, body) = handle(&state_err(), "GET", "/health", "");
        assert_eq!(code, 200);
        assert_eq!(body, json!({ "ok": true, "version": "9.9.9-test" }));
    }

    #[test]
    fn settings_reports_scale_factor() {
        let mut st = state_err();
        st.settings.ui.scale_factor = 1.25;
        let (code, body) = handle(&st, "GET", "/settings", "");
        assert_eq!(code, 200);
        assert_eq!(body, json!({ "scale_factor": 1.25 }));
    }

    #[test]
    fn status_with_db_error_has_error_state_and_zero_counts() {
        let (code, body) = handle(&state_err(), "GET", "/status", "");
        assert_eq!(code, 200);
        assert_eq!(body["version"], "9.9.9-test");
        assert!(body["schema_version"].is_null());
        assert_eq!(body["db"]["state"], "error");
        assert_eq!(body["db"]["message"], "cannot open /x/finances.db: file is not a database");
        for k in ["transactions", "accounts", "categories"] {
            assert_eq!(body["db"][k], 0);
        }
        assert!(body["db"]["last_txn_date"].is_null());
        assert!(body["db"]["last_sync"].is_null());
        assert_eq!(body["mcp"], json!({ "enabled": false }));
    }

    #[test]
    fn transactions_without_a_db_is_503() {
        let (code, body) = handle(&state_err(), "GET", "/transactions", "limit=5");
        assert_eq!(code, 503);
        assert!(body["error"].is_string());
    }

    #[test]
    fn unknown_paths_are_404_for_any_method() {
        for (m, p) in [
            ("GET", "/nope"),
            ("POST", "/nope"),
            ("GET", "/health/"),
            ("GET", "/healthz"),
            ("GET", "/"),
            ("GET", "/transactions/1"),
            ("GET", "/x/health"),
        ] {
            let (code, body) = handle(&state_err(), m, p, "");
            assert_eq!(code, 404, "{} {}", m, p);
            assert_eq!(body, json!({ "error": "not found" }));
        }
    }

    #[test]
    fn known_paths_with_other_methods_are_405() {
        for m in ["POST", "PUT", "DELETE", "PATCH", "HEAD", "get"] {
            for p in ["/health", "/status", "/transactions", "/settings"] {
                let (code, body) = handle(&state_err(), m, p, "");
                assert_eq!(code, 405, "{} {}", m, p);
                assert_eq!(body, json!({ "error": "method not allowed" }));
            }
        }
    }

    #[test]
    fn route_names_for_logs() {
        assert_eq!(route_name("/status"), "status");
        assert_eq!(route_name("/transactions"), "transactions");
        assert_eq!(route_name("/secret/../x"), "unknown");
    }

    #[test]
    fn query_param_takes_the_first_pair() {
        assert_eq!(query_param("limit=5&offset=2", "limit"), Some("5"));
        assert_eq!(query_param("a=1&limit=7&limit=9", "limit"), Some("7"));
        assert_eq!(query_param("limitx=3", "limit"), None);
        assert_eq!(query_param("", "limit"), None);
        assert_eq!(query_param("limit", "limit"), None);
    }
}
