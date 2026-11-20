//! The UI's JSON API: a pure router, so it can be tested without sockets.
//!
//! Paths are matched exactly (no prefix or substring matching), except `/transactions/<id>`, `/views/<id>`
//! and `/categories/<id>/<action>`. Unknown paths are 404; a known path with the wrong method is 405
//! (writes: `POST /refresh`, `POST /views`, `DELETE /views/<id>`, `POST /history`, and the category, categorize
//! and undo/redo routes in `edit.rs`).
//! Data routes read the in-memory `Cache`: 503 when the database is unavailable or the cache has not
//! been loaded. Bodies are `{"error": "..."}` on failure. Errors never carry payee, amount or token data.

use crate::cache::{AccountRow, CatNode, Cache, Txn};
use crate::config::Settings;
use crate::db::Db;
use crate::query::{self, Query};
use crate::search;
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

mod edit;
mod insight;
mod slash;
mod smart;

/// A status code and its JSON body.
pub(crate) type Resp = (u16, Value);

/// Everything the routes read. Built once at startup; shared by the workers.
pub struct AppState {
    pub settings: Settings,
    /// The database, or why it could not be opened (shown by `/status`).
    pub db: Result<Db, String>,
    pub version: &'static str,
    /// The in-memory mirror; `None` until the first successful load.
    cache: RwLock<Option<Cache>>,
    /// Where `search_history.json` lives; `None` means history routes answer 503.
    pub config_dir: Option<PathBuf>,
    /// Fixed "today" for relative dates (tests); `None` uses the local date.
    today_override: Option<NaiveDate>,
    /// Serialises history read-modify-write.
    history_lock: Mutex<()>,
    /// Phase 5 derived model for the current cache; cleared (under the cache write lock) on every refresh.
    insight: Mutex<Option<Arc<crate::insight::Model>>>,
}

impl AppState {
    pub fn new(settings: Settings, db: Result<Db, String>, version: &'static str) -> AppState {
        AppState { settings, db, version, cache: RwLock::new(None), config_dir: None, today_override: None, history_lock: Mutex::new(()), insight: Mutex::new(None) }
    }

    /// Set the directory that holds the search history file.
    pub fn with_config_dir(mut self, dir: PathBuf) -> AppState {
        self.config_dir = Some(dir);
        self
    }

    /// Pin "today" (relative dates in slash search); for tests.
    pub fn with_today(mut self, today: NaiveDate) -> AppState {
        self.today_override = Some(today);
        self
    }

    /// The date relative terms resolve against: the local date unless pinned.
    pub fn today(&self) -> NaiveDate {
        self.today_override.unwrap_or_else(|| chrono::Local::now().date_naive())
    }

    /// Reload every transaction, account and category from the database and swap the cache in.
    /// Returns the number of transactions loaded. The old cache keeps serving until the swap.
    pub fn refresh(&self) -> Result<usize> {
        let db = self.db.as_ref().map_err(|e| anyhow!("database unavailable: {}", e))?;
        let cache = Cache::load(db)?;
        let n = cache.txns.len();
        {
            let mut w = self.cache.write().unwrap_or_else(|e| e.into_inner());
            *w = Some(cache);
            // Cleared while the write lock is held, so a reader never pairs the new cache with the old model.
            *self.insight.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
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
        Route::CategoryAction(_, _) => "category_action",
        Route::Categorize => "categorize",
        Route::Undo => "undo",
        Route::Redo => "redo",
        Route::Journal => "journal",
        Route::Refresh => "refresh",
        Route::SearchParse => "search_parse",
        Route::Views => "views",
        Route::View(_) => "view",
        Route::History => "history",
        Route::SmartPreview => "smart_preview",
        Route::SmartRows => "smart_rows",
        Route::SmartApply => "smart_apply",
        Route::Rules => "rules",
        Route::Suggest => "suggest",
        Route::SuggestAccuracy => "suggest_accuracy",
        Route::Review => "review",
        Route::InsightsRecurring => "insights_recurring",
        Route::InsightsAnomalies => "insights_anomalies",
        Route::InsightsTransfers => "insights_transfers",
        Route::Dashboard => "dashboard",
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
    CategoryAction(&'a str, &'a str),
    Categorize,
    Undo,
    Redo,
    Journal,
    Refresh,
    SearchParse,
    Views,
    View(&'a str),
    History,
    SmartPreview,
    SmartRows,
    SmartApply,
    Rules,
    Suggest,
    SuggestAccuracy,
    Review,
    InsightsRecurring,
    InsightsAnomalies,
    InsightsTransfers,
    Dashboard,
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
        "/transactions/categorize" => Route::Categorize,
        "/undo" => Route::Undo,
        "/redo" => Route::Redo,
        "/journal" => Route::Journal,
        "/refresh" => Route::Refresh,
        "/search/parse" => Route::SearchParse,
        "/views" => Route::Views,
        "/history" => Route::History,
        "/smart/preview" => Route::SmartPreview,
        "/smart/rows" => Route::SmartRows,
        "/smart/apply" => Route::SmartApply,
        "/rules" => Route::Rules,
        "/suggest" => Route::Suggest,
        "/suggest/accuracy" => Route::SuggestAccuracy,
        "/review" => Route::Review,
        "/insights/recurring" => Route::InsightsRecurring,
        "/insights/anomalies" => Route::InsightsAnomalies,
        "/insights/transfers" => Route::InsightsTransfers,
        "/dashboard" => Route::Dashboard,
        _ => {
            if let Some(rest) = path.strip_prefix("/transactions/") {
                if !rest.is_empty() && !rest.contains('/') {
                    return Route::Transaction(rest);
                }
            } else if let Some(rest) = path.strip_prefix("/categories/") {
                if let Some((id, action)) = rest.split_once('/') {
                    let known = matches!(action, "rename" | "kind" | "move" | "shift" | "delete" | "merge");
                    if !id.is_empty() && !id.contains('/') && known {
                        return Route::CategoryAction(id, action);
                    }
                }
            } else if let Some(rest) = path.strip_prefix("/views/") {
                if !rest.is_empty() && !rest.contains('/') {
                    return Route::View(rest);
                }
            }
            Route::Unknown
        }
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

/// Route one request without a body (see `handle_body`).
pub fn handle(state: &AppState, method: &str, path: &str, query: &str) -> (u16, Value) {
    handle_body(state, method, path, query, b"")
}

/// Route one request. Returns the status code and the JSON body. `body` is the raw request body
/// (at most `http::MAX_BODY` bytes); only `POST /views` and `POST /history` read it.
pub fn handle_body(state: &AppState, method: &str, path: &str, query: &str, body: &[u8]) -> (u16, Value) {
    let r = route(path);
    let allowed: &[&str] = match r {
        Route::Unknown => return error(404, "not found"),
        Route::Refresh | Route::CategoryAction(_, _) | Route::Categorize | Route::Undo | Route::Redo => {
            &["POST"]
        }
        Route::SmartPreview | Route::SmartRows | Route::SmartApply | Route::Suggest => &["POST"],
        Route::Categories => &["GET", "POST"],
        Route::Journal => &["GET"],
        Route::Views | Route::History => &["GET", "POST"],
        Route::View(_) => &["DELETE"],
        _ => &["GET"],
    };
    if !allowed.contains(&method) {
        return error(405, "method not allowed");
    }
    match r {
        Route::Health => (200, json!({ "ok": true, "version": state.version })),
        Route::Status => (200, status(state)),
        Route::Settings => (
            200,
            json!({ "scale_factor": state.settings.ui.scale_factor, "max_depth": state.settings.categories.max_depth.clamp(1, 6) }),
        ),
        Route::Transactions => transactions(state, query),
        Route::Transaction(id) => transaction(state, id),
        Route::Accounts => accounts(state),
        Route::Categories if method == "POST" => edit::category_create(state, body),
        Route::Categories => categories(state),
        Route::CategoryAction(id, action) => edit::category_action(state, id, action, body),
        Route::Categorize => edit::categorize(state, body),
        Route::Undo => edit::undo(state),
        Route::Redo => edit::redo(state),
        Route::Journal => edit::journal(state, query),
        Route::Refresh => refresh(state),
        Route::SearchParse => slash::search_parse(state, query),
        Route::Views if method == "POST" => slash::view_save(state, body),
        Route::Views => slash::views_list(state),
        Route::View(id) => slash::view_delete(state, id),
        Route::History if method == "POST" => slash::history_add(state, body),
        Route::History => slash::history_list(state),
        Route::SmartPreview => smart::preview(state, body),
        Route::SmartRows => smart::rows(state, body),
        Route::SmartApply => smart::apply(state, body),
        Route::Rules => smart::rules(state),
        Route::Suggest => insight::suggest(state, body),
        Route::SuggestAccuracy => insight::suggest_accuracy(state, query),
        Route::Review => insight::review(state, query),
        Route::InsightsRecurring => insight::recurring(state, query),
        Route::InsightsAnomalies => insight::anomalies(state, query),
        Route::InsightsTransfers => insight::transfers(state, query),
        Route::Dashboard => insight::dashboard(state, query),
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
    let (rest, s) = match slash::take_search(qs) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut q = match Query::parse(&rest) {
        Ok(q) => q,
        Err(msg) => return error(400, &msg),
    };
    let search = match s.as_deref().map(|s| search::parse(s, state.today())) {
        None => None,
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => return slash::parse_error_body(&e),
    };
    // A sort directive in the search applies only when the request does not sort itself.
    if let Some((key, dir)) = search.as_ref().and_then(|s| s.sort) {
        let explicit = rest.split('&').any(|kv| matches!(kv.split_once('=').map_or(kv, |(k, _)| k), "sort" | "dir"));
        if !explicit {
            q.sort = key.to_query();
            q.desc = dir == search::Dir::Desc;
        }
    }
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return unavailable();
    };
    let compiled = search.as_ref().filter(|s| !s.terms.is_empty()).map(|s| s.compile(cache));
    let pred = compiled.as_ref().map(|c| move |t: &Txn| c.matches(t));
    let r = query::run_with(cache, &q, pred.as_ref().map(|p| p as &dyn Fn(&Txn) -> bool));
    let rows: Vec<Value> = r.rows.iter().map(|t| txn_row(cache, t)).collect();
    let mut body = json!({
        "all": cache.txns.len(),
        "total": r.total,
        "sum_out": money(r.sum_out),
        "sum_in": money(r.sum_in),
        "rows": rows,
    });
    if let (Some(s), Some(o)) = (&search, body.as_object_mut()) {
        o.insert("search".to_string(), json!({ "canonical": s.canonical() }));
    }
    (200, body)
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
        "sort": c.sort,
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
