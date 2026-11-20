//! Phase 5 routes: `POST /suggest`, `GET /suggest/accuracy`, `GET /review`, `GET /insights/{recurring,anomalies,transfers}`,
//! `GET /dashboard`. Read-only over the cache; the derived model lives in `AppState` and is dropped on refresh.
//! Errors use the `{error, message}` envelope. Unknown query keys are 400.

use super::{AppState, Resp};
use crate::cache::Cache;
use crate::insight::{self, dashboard, suggest, Model};
use serde_json::{json, Value};
use std::sync::Arc;

fn envelope(status: u16, code: &str, message: &str) -> Resp {
    (status, json!({ "error": code, "message": message }))
}

fn invalid(message: &str) -> Resp {
    envelope(400, "invalid", message)
}

fn unavailable() -> Resp {
    envelope(503, "unavailable", "database unavailable")
}

fn check_db(state: &AppState) -> Result<(), Resp> {
    state.db.as_ref().map(|_| ()).map_err(|_| unavailable())
}

/// `key=value` pairs; keys must be in `allowed`, each at most once.
fn params<'a>(qs: &'a str, allowed: &[&str]) -> Result<Vec<(&'a str, &'a str)>, Resp> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for kv in qs.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if !allowed.contains(&k) {
            return Err(invalid(&format!("unknown parameter {k}")));
        }
        if out.iter().any(|(x, _)| *x == k) {
            return Err(invalid(&format!("{k} given twice")));
        }
        out.push((k, v));
    }
    Ok(out)
}

fn uint(v: &str, name: &str, min: usize, max: usize) -> Result<usize, Resp> {
    match v.parse::<usize>() {
        Ok(n) if v.bytes().all(|b| b.is_ascii_digit()) && (min..=max).contains(&n) => Ok(n),
        _ => Err(invalid(&format!("{name} must be an integer from {min} to {max}"))),
    }
}

/// The derived model for the current cache, built on first use. Called with the cache read lock held, which
/// keeps `refresh` (write lock, then it clears the model) from interleaving.
fn model(state: &AppState, cache: &Cache) -> Result<Arc<Model>, Resp> {
    let mut slot = state.insight.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = slot.as_ref() {
        return Ok(m.clone());
    }
    let Ok(db) = state.db.as_ref() else { return Err(unavailable()) };
    let aliases = db.payee_aliases().map_err(|e| {
        log::error!("insight: aliases: {:#}", e);
        envelope(500, "failed", "database read failed")
    })?;
    let rules = db.list_rules().map_err(|e| {
        log::error!("insight: rules: {:#}", e);
        envelope(500, "failed", "database read failed")
    })?;
    let m = Arc::new(Model::build(cache, &aliases, &rules));
    *slot = Some(m.clone());
    Ok(m)
}

fn gate(s: &suggest::Suggest) -> Value {
    json!({ "enabled": s.enabled, "reason": s.reason, "accuracy": s.accuracy })
}

/// Runs `f` with the cache and model, or answers 503.
fn with_model(state: &AppState, f: impl FnOnce(&Cache, &Model) -> Resp) -> Resp {
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else { return unavailable() };
    match model(state, cache) {
        Ok(m) => f(cache, &m),
        Err(e) => e,
    }
}

pub(super) fn suggest(state: &AppState, body: &[u8]) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    let ids: Vec<i64> = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => match m.get("ids") {
            Some(Value::Array(a)) if !a.is_empty() && a.len() <= 200 => {
                match a.iter().map(|v| v.as_i64()).collect::<Option<Vec<_>>>() {
                    Some(v) => v,
                    None => return invalid("ids must be integers"),
                }
            }
            Some(Value::Array(_)) => return invalid("ids must hold 1 to 200 transaction ids"),
            _ => return invalid("ids must be an array of integers"),
        },
        _ => return invalid("request body must be a JSON object"),
    };
    with_model(state, |cache, model| {
        let s = model.suggest(cache);
        let mut body = gate(s);
        let mut out: Vec<Value> = Vec::new();
        if s.enabled {
            let mut seen: Vec<i64> = Vec::new();
            for id in ids {
                if seen.contains(&id) {
                    continue;
                }
                seen.push(id);
                let Ok(i) = cache.txns.binary_search_by_key(&id, |t| t.id) else { continue };
                if cache.txns[i].category_id.is_some() {
                    continue;
                }
                let options = suggest::options(model, cache, i, true);
                out.push(json!({ "id": id, "options": options }));
            }
        }
        body["suggestions"] = json!(out);
        (200, body)
    })
}

pub(super) fn suggest_accuracy(state: &AppState, qs: &str) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    if let Err(e) = params(qs, &[]) {
        return e;
    }
    with_model(state, |cache, model| (200, gate(model.suggest(cache))))
}

pub(super) fn review(state: &AppState, qs: &str) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    let (mut limit, mut offset) = (50usize, 0usize);
    let ps = match params(qs, &["limit", "offset"]) {
        Ok(p) => p,
        Err(e) => return e,
    };
    for (k, v) in ps {
        let r = if k == "limit" { uint(v, "limit", 1, insight::review::MAX_LIMIT).map(|n| limit = n) } else { uint(v, "offset", 0, usize::MAX >> 1).map(|n| offset = n) };
        if let Err(e) = r {
            return e;
        }
    }
    with_model(state, |cache, model| {
        let q = insight::review::queue(cache, model, limit, offset);
        (200, insight::review::report(cache, &q))
    })
}

pub(super) fn recurring(state: &AppState, qs: &str) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    if let Err(e) = params(qs, &[]) {
        return e;
    }
    let today = state.today();
    with_model(state, |cache, model| {
        (200, insight::recurring::report(&insight::recurring::detect(cache, model, today)))
    })
}

pub(super) fn anomalies(state: &AppState, qs: &str) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    if let Err(e) = params(qs, &[]) {
        return e;
    }
    let today = state.today();
    with_model(state, |cache, model| {
        (200, insight::anomalies::report(cache, &insight::anomalies::detect(cache, model, today)))
    })
}

pub(super) fn transfers(state: &AppState, qs: &str) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    if let Err(e) = params(qs, &[]) {
        return e;
    }
    with_model(state, |cache, model| (200, insight::transfers::report(cache, &model.transfers)))
}

pub(super) fn dashboard(state: &AppState, qs: &str) -> Resp {
    if let Err(e) = check_db(state) {
        return e;
    }
    let mut p = dashboard::Params { months: 12, exclude_transfers: true };
    let ps = match params(qs, &["months", "exclude_transfers"]) {
        Ok(p) => p,
        Err(e) => return e,
    };
    for (k, v) in ps {
        if k == "months" {
            match uint(v, "months", 1, dashboard::MAX_MONTHS as usize) {
                Ok(n) => p.months = n as u32,
                Err(e) => return e,
            }
        } else {
            p.exclude_transfers = match v {
                "true" => true,
                "false" => false,
                _ => return invalid("exclude_transfers must be true or false"),
            };
        }
    }
    let today = state.today();
    with_model(state, |cache, model| (200, dashboard::report(cache, model, today, &p)))
}
