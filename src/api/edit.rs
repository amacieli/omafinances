//! Write routes for the category tree, manual categorization and the undo/redo journal. Every body is a
//! JSON object. Errors are `{"error": code, "message": text}` with codes `invalid` (400), `not_found` (404),
//! `conflict` / `limit` / `nothing_to_undo` / `nothing_to_redo` (409). On success the in-memory cache is
//! reloaded before the response is sent, so the next GET already shows the change.

use super::{AppState, Resp};
use crate::db::categories::{Categorized, MAX_CATEGORY_IDS};
use crate::db::journal::{MutError, Outcome};
use crate::db::Db;
use serde_json::{json, Map, Value};

fn envelope(status: u16, code: &str, message: &str) -> Resp {
    (status, json!({ "error": code, "message": message }))
}

fn invalid(message: &str) -> Resp {
    envelope(400, "invalid", message)
}

fn mut_error(e: MutError) -> Resp {
    match e {
        MutError::Invalid(m) => invalid(&m),
        MutError::NotFound(m) => envelope(404, "not_found", m),
        MutError::Conflict(m) => envelope(409, "conflict", &m),
        MutError::Limit(m) => envelope(409, "limit", &m),
        MutError::NothingToUndo => envelope(409, "nothing_to_undo", "nothing to undo"),
        MutError::NothingToRedo => envelope(409, "nothing_to_redo", "nothing to redo"),
        MutError::Other(e) => {
            // The detail is logged, not returned: it can carry row text.
            log::error!("edit: {:#}", e);
            envelope(500, "failed", "database write failed")
        }
    }
}

fn db_of(state: &AppState) -> Result<&Db, Resp> {
    state.db.as_ref().map_err(|_| envelope(503, "unavailable", "database unavailable"))
}

fn max_depth(state: &AppState) -> u32 {
    state.settings.categories.max_depth.clamp(1, 6)
}

/// Run a write, reload the cache, and shape the success body.
fn commit<T>(state: &AppState, r: Result<T, MutError>, shape: impl FnOnce(T) -> Value) -> Resp {
    match r {
        Ok(v) => {
            if let Err(e) = state.refresh() {
                log::error!("edit: reload after write failed: {:#}", e);
                return envelope(500, "refresh_failed", "the change was saved but the view could not be reloaded; refresh to see it");
            }
            (200, shape(v))
        }
        Err(e) => mut_error(e),
    }
}

fn outcome_json(o: Outcome) -> Value {
    json!({ "ok": true, "batch_id": o.batch_id, "label": o.label })
}

/// Parse a path id: an optional minus and digits, nothing else.
pub(super) fn path_id(raw: &str, what: &str) -> Result<i64, Resp> {
    match raw.parse::<i64>() {
        Ok(v) if !raw.starts_with('+') => Ok(v),
        _ => Err(invalid(&format!("{what} must be an integer"))),
    }
}

fn parse_body(body: &[u8]) -> Result<Map<String, Value>, Resp> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(invalid("request body must be a JSON object")),
    }
}

/// Required key whose value is an integer or null (`parent_id`, `reassign_to`, `category_id`).
fn nullable_int(obj: &Map<String, Value>, key: &str) -> Result<Option<i64>, Resp> {
    match obj.get(key) {
        None => Err(invalid(&format!("{key} is required (an integer or null)"))),
        Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().map(Some).ok_or_else(|| invalid(&format!("{key} must be an integer or null"))),
    }
}

fn required_int(obj: &Map<String, Value>, key: &str) -> Result<i64, Resp> {
    match obj.get(key) {
        None | Some(Value::Null) => Err(invalid(&format!("{key} is required"))),
        Some(v) => v.as_i64().ok_or_else(|| invalid(&format!("{key} must be an integer"))),
    }
}

fn required_str(obj: &Map<String, Value>, key: &str) -> Result<String, Resp> {
    match obj.get(key) {
        None | Some(Value::Null) => Err(invalid(&format!("{key} is required"))),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(invalid(&format!("{key} must be a string"))),
    }
}

fn optional_str(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, Resp> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(&format!("{key} must be a string"))),
    }
}

// ---- categories -----------------------------------------------------------------------------------

pub(super) fn category_create(state: &AppState, body: &[u8]) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let obj = match parse_body(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let parse = || -> Result<(Option<i64>, String, Option<String>), Resp> {
        Ok((nullable_int(&obj, "parent_id")?, required_str(&obj, "name")?, optional_str(&obj, "kind")?))
    };
    let (parent, name, kind) = match parse() {
        Ok(v) => v,
        Err(e) => return e,
    };
    let r = db.create_category(max_depth(state), parent, &name, kind.as_deref());
    commit(state, r, |o| json!({ "ok": true, "id": o.id, "batch_id": o.batch_id, "label": o.label }))
}

pub(super) fn category_action(state: &AppState, id: &str, action: &str, body: &[u8]) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let id = match path_id(id, "category id") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let obj = match parse_body(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let depth = max_depth(state);
    match action {
        "rename" => {
            let name = match required_str(&obj, "name") {
                Ok(n) => n,
                Err(e) => return e,
            };
            commit(state, db.rename_category(id, &name), outcome_json)
        }
        "kind" => {
            let kind = match required_str(&obj, "kind") {
                Ok(k) => k,
                Err(e) => return e,
            };
            commit(state, db.set_category_kind(id, &kind), outcome_json)
        }
        "move" => {
            let parent = match nullable_int(&obj, "parent_id") {
                Ok(p) => p,
                Err(e) => return e,
            };
            let position = match obj.get("position") {
                None | Some(Value::Null) => None,
                Some(v) => match v.as_i64().and_then(|n| usize::try_from(n).ok()) {
                    Some(n) => Some(n),
                    None => return invalid("position must be a non-negative integer"),
                },
            };
            commit(state, db.move_category(depth, id, parent, position), outcome_json)
        }
        "shift" => {
            let delta = match required_int(&obj, "delta") {
                Ok(d) => d,
                Err(e) => return e,
            };
            if delta != -1 && delta != 1 {
                return invalid("delta must be -1 or 1");
            }
            commit(state, db.move_category_within(depth, id, delta), outcome_json)
        }
        "delete" => {
            let reassign = match nullable_int(&obj, "reassign_to") {
                Ok(r) => r,
                Err(e) => return e,
            };
            commit(state, db.delete_category(id, reassign), outcome_json)
        }
        "merge" => {
            let into = match required_int(&obj, "into") {
                Ok(i) => i,
                Err(e) => return e,
            };
            commit(state, db.merge_category(depth, id, into), outcome_json)
        }
        _ => envelope(404, "not_found", "not found"),
    }
}

// ---- manual categorization ------------------------------------------------------------------------

pub(super) fn categorize(state: &AppState, body: &[u8]) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let obj = match parse_body(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let ids: Vec<i64> = match obj.get("ids") {
        Some(Value::Array(items)) => {
            if items.len() > MAX_CATEGORY_IDS {
                return invalid(&format!("at most {MAX_CATEGORY_IDS} transaction ids per call"));
            }
            let mut out = Vec::with_capacity(items.len());
            for v in items {
                match v.as_i64() {
                    Some(i) => out.push(i),
                    None => return invalid("ids must be integers"),
                }
            }
            out
        }
        _ => return invalid("ids must be an array of integers"),
    };
    let category = match nullable_int(&obj, "category_id") {
        Ok(c) => c,
        Err(e) => return e,
    };
    commit(state, db.categorize(&ids, category), |c: Categorized| {
        json!({ "ok": true, "batch_id": c.batch_id, "changed": c.changed, "skipped": c.skipped, "label": c.label })
    })
}

// ---- journal --------------------------------------------------------------------------------------

pub(super) fn undo(state: &AppState) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    commit(state, db.undo(), outcome_json)
}

pub(super) fn redo(state: &AppState) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    commit(state, db.redo(), outcome_json)
}

/// `?limit=` in 1..=100, default 20. Other parameters, duplicates and non-digits are errors.
pub(super) fn journal(state: &AppState, qs: &str) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let mut limit = 20i64;
    let mut seen = false;
    for kv in qs.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k != "limit" || seen {
            return invalid("unknown or repeated parameter (only limit is accepted)");
        }
        seen = true;
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
            return invalid("limit must be an integer from 1 to 100");
        }
        limit = match v.parse::<i64>() {
            Ok(n) if (1..=100).contains(&n) => n,
            _ => return invalid("limit must be an integer from 1 to 100"),
        };
    }
    match db.journal(limit) {
        Ok(view) => (200, serde_json::to_value(view).unwrap_or_else(|_| json!({ "rows": [] }))),
        Err(e) => {
            log::error!("journal: {:#}", e);
            envelope(500, "failed", "database query failed")
        }
    }
}
