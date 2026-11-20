//! Routes added by slash search: `/search/parse`, `/views`, `/views/<id>`, `/history`.
//! Query text is private: nothing here logs it, and error bodies never echo it (parse errors carry a
//! message about the offending text, as the UI needs it to underline the mistake).

use super::{error, unavailable, AppState};
use crate::search::{self, complete, history, ParseError};
use serde_json::{json, Map, Value};

/// Longest saved-view name, in characters.
pub const MAX_NAME: usize = 60;

pub(super) fn parse_error_body(e: &ParseError) -> (u16, Value) {
    (400, json!({ "error": "parse", "parse_error": e.to_json() }))
}

/// The body must be a JSON object (no Content-Type needed).
fn json_object(body: &[u8]) -> Result<Map<String, Value>, (u16, Value)> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(error(400, "request body must be a JSON object")),
    }
}

/// Split a query string into decoded `(key, value)` pairs for the routes that take free text.
/// Unknown or duplicate keys and bad escapes are errors.
fn params<'a>(qs: &'a str, known: &[&str]) -> Result<Vec<(&'a str, String)>, (u16, Value)> {
    let mut out: Vec<(&str, String)> = Vec::new();
    for kv in qs.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if !known.contains(&k) {
            return Err(error(400, "unknown parameter"));
        }
        if out.iter().any(|(ok, _)| *ok == k) {
            return Err(error(400, &format!("duplicate parameter: {}", k)));
        }
        let v = crate::query::percent_decode(v).map_err(|_| error(400, &format!("invalid value for {}", k)))?;
        out.push((k, v));
    }
    Ok(out)
}

pub(super) fn search_parse(state: &AppState, qs: &str) -> (u16, Value) {
    let p = match params(qs, &["q", "pos"]) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let get = |k: &str| p.iter().find(|(pk, _)| *pk == k).map(|(_, v)| v.as_str());
    let q = get("q").unwrap_or("");
    let pos = match get("pos") {
        None => q.len(),
        Some(v) => match v.parse::<usize>() {
            Ok(n) if v.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => return error(400, "pos must be a non-negative integer"),
        },
    };
    let today = state.today();
    let guard = state.read_cache();
    let cache = guard.as_ref();
    let completions = complete::complete(cache, q, pos, today).to_json();

    let body = match search::parse(q, today) {
        Ok(s) => {
            let terms: Vec<Value> = s
                .terms
                .iter()
                .map(|t| {
                    let (field, value) = t.parts();
                    json!({ "text": t.text(), "neg": t.neg, "field": field.map_or("text", |f| f.long()), "value": value })
                })
                .collect();
            let matched = cache.map(|c| {
                let compiled = s.compile(c);
                c.txns.iter().filter(|t| compiled.matches(t)).count()
            });
            json!({
                "ok": true, "canonical": s.canonical(), "terms": terms, "sort": s.sort_text(),
                "parse_error": null, "matched": matched, "completions": completions,
            })
        }
        Err(e) => json!({
            "ok": false, "canonical": null, "terms": [], "sort": null,
            "parse_error": e.to_json(), "matched": null, "completions": completions,
        }),
    };
    (200, body)
}

// ---- saved views ------------------------------------------------------------------------------

pub(super) fn views_list(state: &AppState) -> (u16, Value) {
    let Ok(db) = state.db.as_ref() else {
        return unavailable();
    };
    match db.views() {
        Ok(v) => (200, json!({ "rows": v.iter().map(|v| v.to_json()).collect::<Vec<_>>() })),
        Err(e) => {
            log::error!("views: database query failed: {:#}", e);
            error(500, "database query failed")
        }
    }
}

pub(super) fn view_save(state: &AppState, body: &[u8]) -> (u16, Value) {
    let Ok(db) = state.db.as_ref() else {
        return unavailable();
    };
    let obj = match json_object(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let text = |k: &str| -> Result<Option<&str>, (u16, Value)> {
        match obj.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(error(400, &format!("{} must be a string", k))),
        }
    };
    let name = match text("name") {
        Ok(Some(n)) => n.trim(),
        Ok(None) => return error(400, "name is required"),
        Err(e) => return e,
    };
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return error(400, "name must be 1 to 60 characters with no control characters");
    }
    let query = match text("query") {
        Ok(Some(q)) => q.trim(),
        Ok(None) => return error(400, "query is required"),
        Err(e) => return e,
    };
    if query.chars().count() > history::MAX_LEN {
        return error(400, "query is too long");
    }
    let sort = match text("sort") {
        Ok(s) => s.filter(|s| !s.is_empty()),
        Err(e) => return e,
    };
    if let Some(s) = sort {
        let key = s.strip_prefix('-').unwrap_or(s);
        if search::SortKey::from_name(key).is_none() {
            return error(400, "sort must be date, amount, payee, account or category, optionally with a leading -");
        }
    }
    if let Err(e) = search::parse(query, state.today()) {
        return parse_error_body(&e);
    }
    match db.view_save(name, query, sort) {
        Ok(v) => (200, v.to_json()),
        Err(e) => {
            log::error!("views: save failed: {:#}", e);
            error(500, "database write failed")
        }
    }
}

pub(super) fn view_delete(state: &AppState, id: &str) -> (u16, Value) {
    let Ok(db) = state.db.as_ref() else {
        return unavailable();
    };
    let id: i64 = match id.parse::<i64>() {
        Ok(v) if !id.starts_with('+') => v,
        _ => return error(400, "view id must be an integer"),
    };
    match db.view_delete(id) {
        Ok(true) => (200, json!({ "ok": true })),
        Ok(false) => error(404, "view not found"),
        Err(e) => {
            log::error!("views: delete failed: {:#}", e);
            error(500, "database write failed")
        }
    }
}

// ---- history ----------------------------------------------------------------------------------

pub(super) fn history_list(state: &AppState) -> (u16, Value) {
    let Some(dir) = state.config_dir.as_deref() else {
        return error(503, "history unavailable");
    };
    let _g = state.history_lock.lock().unwrap_or_else(|e| e.into_inner());
    (200, json!({ "rows": history::load(dir) }))
}

pub(super) fn history_add(state: &AppState, body: &[u8]) -> (u16, Value) {
    let Some(dir) = state.config_dir.as_deref() else {
        return error(503, "history unavailable");
    };
    let obj = match json_object(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let Some(Value::String(q)) = obj.get("q") else {
        return error(400, "q must be a string");
    };
    if q.chars().count() > history::MAX_LEN {
        return error(400, "query is too long");
    }
    let q = q.trim();
    // Empty and unparsable queries are not worth remembering; that is not a client error.
    if q.is_empty() || search::parse(q, state.today()).is_err() {
        return (200, json!({ "ok": true, "added": false }));
    }
    let _g = state.history_lock.lock().unwrap_or_else(|e| e.into_inner());
    match history::add(dir, q) {
        Ok(_) => (200, json!({ "ok": true, "added": true })),
        Err(e) => {
            log::error!("history: write failed: {:#}", e);
            error(500, "history write failed")
        }
    }
}

/// Pull `s=<query>` out of a `/transactions` query string. Returns the remaining string for
/// `Query::parse` and the decoded search text, if any.
pub(super) fn take_search(qs: &str) -> Result<(String, Option<String>), (u16, Value)> {
    let mut rest: Vec<&str> = Vec::new();
    let mut s: Option<String> = None;
    for kv in qs.split('&').filter(|x| !x.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k == "s" {
            if s.is_some() {
                return Err(error(400, "duplicate parameter: s"));
            }
            s = Some(crate::query::percent_decode(v).map_err(|_| error(400, "invalid value for s"))?);
        } else {
            rest.push(kv);
        }
    }
    Ok((rest.join("&"), s))
}
