//! The UI's JSON API: a pure router, so it can be tested without sockets.
//!
//! Paths are matched exactly (no prefix or substring matching), except `/transactions/<id>`.
//! Unknown paths are 404; a known path with the wrong method is 405 (`POST` only on `/refresh`).
//! Data routes read the in-memory `Cache`: 503 when the database is unavailable or the cache has not
//! been loaded. Bodies are `{"error": "..."}` on failure. Errors never carry payee, amount or token data.

use crate::cache::{AccountRow, CatNode, Cache, Txn};
use crate::config::Settings;
use crate::db::Db;
use crate::query::{self, Query};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::{RwLock, RwLockReadGuard};

/// Everything the routes read. Built once at startup; shared by the workers.
pub struct AppState {
    pub settings: Settings,
    /// The database, or why it could not be opened (shown by `/status`).
    pub db: Result<Db, String>,
    pub version: &'static str,
    /// The in-memory mirror; `None` until the first successful load.
    cache: RwLock<Option<Cache>>,
}

impl AppState {
    pub fn new(settings: Settings, db: Result<Db, String>, version: &'static str) -> AppState {
        AppState { settings, db, version, cache: RwLock::new(None) }
    }

    /// Reload every transaction, account and category from the database and swap the cache in.
    /// Returns the number of transactions loaded. The old cache keeps serving until the swap.
    pub fn refresh(&self) -> Result<usize> {
        let db = self.db.as_ref().map_err(|e| anyhow!("database unavailable: {}", e))?;
        let cache = Cache::load(db)?;
        let n = cache.txns.len();
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(cache);
        Ok(n)
    }

    fn read_cache(&self) -> RwLockReadGuard<'_, Option<Cache>> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }
}

/// Route name for logs. Never includes the query string or the id.
pub fn route_name(path: &str) -> &'static str {
    match route(path) {
        Route::Health => "health",
        Route::Status => "status",
        Route::Settings => "settings",
        Route::Transactions => "transactions",
        Route::Transaction(_) => "transaction",
        Route::Accounts => "accounts",
        Route::Categories => "categories",
        Route::Refresh => "refresh",
        Route::Unknown => "unknown",
    }
}

enum Route<'a> {
    Health,
    Status,
    Settings,
    Transactions,
    Transaction(&'a str),
    Accounts,
    Categories,
    Refresh,
    Unknown,
}

fn route(path: &str) -> Route<'_> {
    match path {
        "/health" => Route::Health,
        "/status" => Route::Status,
        "/settings" => Route::Settings,
        "/transactions" => Route::Transactions,
        "/accounts" => Route::Accounts,
        "/categories" => Route::Categories,
        "/refresh" => Route::Refresh,
        _ => match path.strip_prefix("/transactions/") {
            Some(rest) if !rest.is_empty() && !rest.contains('/') => Route::Transaction(rest),
            _ => Route::Unknown,
        },
    }
}

fn error(code: u16, message: &str) -> (u16, Value) {
    (code, json!({ "error": message }))
}

fn unavailable() -> (u16, Value) {
    error(503, "database unavailable")
}

/// Round to cents for display. Sums are kept in f64 while adding; this hides the float dust.
fn money(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Route one request. Returns the status code and the JSON body.
pub fn handle(state: &AppState, method: &str, path: &str, query: &str) -> (u16, Value) {
    let r = route(path);
    let want = match r {
        Route::Unknown => return error(404, "not found"),
        Route::Refresh => "POST",
        _ => "GET",
    };
    if method != want {
        return error(405, "method not allowed");
    }
    match r {
        Route::Health => (200, json!({ "ok": true, "version": state.version })),
        Route::Status => (200, status(state)),
        Route::Settings => (200, json!({ "scale_factor": state.settings.ui.scale_factor })),
        Route::Transactions => transactions(state, query),
        Route::Transaction(id) => transaction(state, id),
        Route::Accounts => accounts(state),
        Route::Categories => categories(state),
        Route::Refresh => refresh(state),
        Route::Unknown => error(404, "not found"),
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

fn txn_row(cache: &Cache, t: &Txn) -> Value {
    json!({
        "id": t.id,
        "date": t.date,
        "payee": t.payee,
        "amount": t.amount,
        "currency": t.currency,
        "account_id": t.account_id,
        "account": t.account,
        "category": cache.category_path(t),
        "category_id": t.category_id,
        "lm_category": t.lm_category,
        "pending": t.pending,
        "recurring": t.recurring,
    })
}

fn transactions(state: &AppState, qs: &str) -> (u16, Value) {
    let q = match Query::parse(qs) {
        Ok(q) => q,
        Err(msg) => return error(400, &msg),
    };
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    let r = query::run(cache, &q);
    let rows: Vec<Value> = r.rows.iter().map(|t| txn_row(cache, t)).collect();
    (
        200,
        json!({
            "all": cache.txns.len(),
            "total": r.total,
            "sum_out": money(r.sum_out),
            "sum_in": money(r.sum_in),
            "rows": rows,
        }),
    )
}

fn transaction(state: &AppState, id: &str) -> (u16, Value) {
    // Strict: an optional minus and digits. `+5` and spaces are not integers here.
    let id: i64 = match id.parse::<i64>() {
        Ok(v) if !id.starts_with('+') => v,
        _ => return error(400, "transaction id must be an integer"),
    };
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    // `txns` is ordered by id (see Db::load_all), so binary search is valid.
    let Ok(i) = cache.txns.binary_search_by_key(&id, |t| t.id) else {
        return error(404, "transaction not found");
    };
    let t = &cache.txns[i];
    let mut body = txn_row(cache, t);
    let extra = json!({
        "original_name": t.original_name,
        "notes": t.notes,
        "status": t.status,
        "source": t.source,
        "tags": t.tags,
        "recurring_id": t.recurring_id,
        "external_id": t.external_id,
        "created_at_lm": t.created_at_lm,
        "updated_at_lm": t.updated_at_lm,
    });
    if let (Some(b), Value::Object(e)) = (body.as_object_mut(), extra) {
        b.extend(e);
    }
    (200, body)
}

fn account_json(a: &AccountRow) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "type": a.kind,
        "institution": a.institution,
        "mask": a.mask,
        "balance": a.balance,
        "currency": a.currency,
        "count": a.count,
        "sum_out": money(a.sum_out),
    })
}

fn accounts(state: &AppState) -> (u16, Value) {
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    let rows: Vec<Value> = cache.accounts.iter().map(account_json).collect();
    (200, json!({ "rows": rows }))
}

fn category_json(c: &CatNode) -> Value {
    json!({
        "id": c.id,
        "parent_id": c.parent_id,
        "name": c.name,
        "kind": c.kind,
        "depth": c.depth,
        "count": c.count,
        "count_subtree": c.count_subtree,
        "sum_out_subtree": money(c.sum_out_subtree),
    })
}

fn categories(state: &AppState) -> (u16, Value) {
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    let rows: Vec<Value> = cache.categories.iter().map(category_json).collect();
    (200, json!({ "rows": rows, "uncategorized": cache.uncategorized }))
}

fn refresh(state: &AppState) -> (u16, Value) {
    if state.db.is_err() {
        return unavailable();
    }
    match state.refresh() {
        Ok(n) => (200, json!({ "ok": true, "transactions": n })),
        Err(e) => {
            log::error!("refresh: {:#}", e);
            error(500, "database query failed")
        }
    }
}

#[cfg(test)]
mod tests;
