//! Categorize-with-preview routes: `POST /smart/preview`, `POST /smart/rows`, `POST /smart/apply`, `GET /rules`.
//! Errors use the `{error, message}` envelope of `api/edit.rs`. Preview and rows read the in-memory cache; apply
//! writes through `Db::smart_apply` and reloads the cache before responding, like the edit routes.

use super::{AppState, Resp};
use crate::db::journal::MutError;
use crate::db::rules::MAX_APPLY_IDS;
use crate::db::Db;
use crate::smart::{self, Request, SmartError, DEFAULT_THRESHOLD, MAX_IDS};
use serde_json::{json, Map, Value};

fn envelope(status: u16, code: &str, message: &str) -> Resp {
    (status, json!({ "error": code, "message": message }))
}

fn invalid(message: &str) -> Resp {
    envelope(400, "invalid", message)
}

fn unavailable() -> Resp {
    envelope(503, "unavailable", "database unavailable")
}

fn db_of(state: &AppState) -> Result<&Db, Resp> {
    state.db.as_ref().map_err(|_| unavailable())
}

fn parse_body(body: &[u8]) -> Result<Map<String, Value>, Resp> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(invalid("request body must be a JSON object")),
    }
}

/// A required array of integer ids, 1 to `max` long.
fn ids_field(obj: &Map<String, Value>, key: &str, max: usize) -> Result<Vec<i64>, Resp> {
    match obj.get(key) {
        Some(Value::Array(items)) => {
            if items.is_empty() || items.len() > max {
                return Err(invalid(&format!("{key} must hold 1 to {max} transaction ids")));
            }
            items
                .iter()
                .map(|v| v.as_i64().ok_or_else(|| invalid(&format!("{key} must be integers"))))
                .collect()
        }
        _ => Err(invalid(&format!("{key} must be an array of integers"))),
    }
}

fn required_int(obj: &Map<String, Value>, key: &str) -> Result<i64, Resp> {
    obj.get(key).and_then(Value::as_i64).ok_or_else(|| invalid(&format!("{key} must be an integer")))
}

fn smart_error(e: SmartError) -> Resp {
    match e {
        SmartError::Invalid(m) => invalid(&m),
        SmartError::NotFound(m) => envelope(404, "not_found", m),
    }
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
            log::error!("smart: {:#}", e);
            envelope(500, "failed", "database write failed")
        }
    }
}

/// `POST /smart/preview`: clusters of rows that look like the selected ones. Read-only.
pub(super) fn preview(state: &AppState, body: &[u8]) -> Resp {
    if let Err(e) = db_of(state) {
        return e;
    }
    let obj = match parse_body(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let ids = match ids_field(&obj, "ids", MAX_IDS) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let category_id = match required_int(&obj, "category_id") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let threshold = match obj.get("threshold") {
        None | Some(Value::Null) => DEFAULT_THRESHOLD,
        Some(v) => match v.as_f64() {
            Some(t) => t,
            None => return invalid("threshold must be a number"),
        },
    };
    if !(0.5..=1.0).contains(&threshold) {
        return invalid("threshold must be between 0.5 and 1.0");
    }
    let overwrite = match obj.get("overwrite") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        _ => return invalid("overwrite must be true or false"),
    };
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let aliases = match db.payee_aliases() {
        Ok(a) => a,
        Err(e) => {
            log::error!("smart: aliases: {:#}", e);
            return envelope(500, "failed", "database read failed");
        }
    };
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    let req = Request { ids, category_id, threshold, overwrite };
    match smart::preview(cache, &aliases, &req) {
        Ok(p) => (200, json!(p)),
        Err(e) => smart_error(e),
    }
}

/// `POST /smart/rows`: rows by id, in request order, to expand a cluster. Unknown ids are omitted.
pub(super) fn rows(state: &AppState, body: &[u8]) -> Resp {
    if let Err(e) = db_of(state) {
        return e;
    }
    let obj = match parse_body(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let ids = match ids_field(&obj, "ids", MAX_IDS) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    let rows: Vec<Value> = ids
        .iter()
        .filter_map(|id| cache.txns.binary_search_by_key(id, |t| t.id).ok().map(|i| &cache.txns[i]))
        .map(|t| {
            json!({
                "id": t.id,
                "date": t.date,
                "payee": t.payee,
                "amount": t.amount,
                "account": t.account,
                "category_path": cache.category_path(t),
            })
        })
        .collect();
    (200, json!({ "rows": rows }))
}

/// `POST /smart/apply`: write the category (and optionally a rule) in one journal batch.
pub(super) fn apply(state: &AppState, body: &[u8]) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let obj = match parse_body(body) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let category_id = match required_int(&obj, "category_id") {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ids = match ids_field(&obj, "txn_ids", MAX_APPLY_IDS) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let overwrite = match obj.get("overwrite") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        _ => return invalid("overwrite must be true or false"),
    };
    let make_rule = match obj.get("make_rule") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => match m.get("name") {
            None | Some(Value::Null) => Some(None),
            Some(Value::String(s)) => Some(Some(s.clone())),
            _ => return invalid("make_rule.name must be a string"),
        },
        _ => return invalid("make_rule must be null or an object"),
    };
    match db.smart_apply(category_id, &ids, overwrite, make_rule) {
        Ok(a) => {
            if let Err(e) = state.refresh() {
                log::error!("smart: reload after write failed: {:#}", e);
                return envelope(
                    500,
                    "refresh_failed",
                    "the change was saved but the view could not be reloaded; refresh to see it",
                );
            }
            (
                200,
                json!({
                    "ok": true,
                    "batch_id": a.batch_id,
                    "label": a.label,
                    "changed": a.changed,
                    "skipped": a.skipped,
                    "skipped_conflicts": a.skipped_conflicts,
                    "rule_id": a.rule_id,
                }),
            )
        }
        Err(e) => mut_error(e),
    }
}

/// `GET /rules`: every rule, in apply order.
pub(super) fn rules(state: &AppState) -> Resp {
    let db = match db_of(state) {
        Ok(d) => d,
        Err(e) => return e,
    };
    match db.list_rules() {
        Ok(rs) => (200, json!({ "rules": rs })),
        Err(e) => {
            log::error!("smart: list rules: {:#}", e);
            envelope(500, "failed", "database read failed")
        }
    }
}
