//! Report routes: `/reports…`, `/schedule/preview`, `/mail/status` (contract: `docs/PHASE6_CONTRACT.md`).
//!
//! Errors are `{"error": <code>, "message": <text>}`; a query that does not parse is `{"error":"parse", …}` with the
//! parser's text and position. Run details are fixed phrases, so no response carries row text.

use super::{AppState, Resp};
use crate::cache::Cache;
use crate::db::reports::ReportFields;
use crate::db::Db;
use crate::insight::Model;
use crate::query;
use crate::report::model::{self, check_name, Period, Report, Shape};
use crate::mail::Mailer;
use crate::report::mailglue::{self, NOT_AUTHENTICATED};
use crate::report::run::{self, Ctx, Options};
use crate::report::schedule::Schedule;
use crate::report::{render, sync_all, units};
use crate::search::ParseError;
use chrono::{Local, NaiveDate, NaiveDateTime};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 200;
const NEXT_RUNS: usize = 3;

fn envelope(status: u16, code: &str, message: &str) -> Resp {
    (status, json!({ "error": code, "message": message }))
}

fn invalid(message: &str) -> Resp {
    envelope(400, "invalid", message)
}

fn not_found() -> Resp {
    envelope(404, "not_found", "report not found")
}

fn internal(message: &str) -> Resp {
    envelope(500, "failed", message)
}

fn duplicate() -> Resp {
    envelope(409, "duplicate_name", "a report with this name already exists")
}

fn not_authenticated() -> Resp {
    envelope(409, "not_authenticated", NOT_AUTHENTICATED)
}

fn parse_error(e: &ParseError) -> Resp {
    let mut j = e.to_json();
    j["error"] = json!("parse");
    j["message"] = json!(e.to_string());
    (400, j)
}

fn iso(dt: &NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn db(state: &AppState) -> Result<&Db, Resp> {
    state.db.as_ref().map_err(|_| envelope(503, "unavailable", "database unavailable"))
}

fn parse_body(body: &[u8]) -> Result<Map<String, Value>, Resp> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(invalid("request body must be a JSON object")),
    }
}

fn parse_id(s: &str) -> Result<i64, Resp> {
    s.parse::<i64>().ok().filter(|n| *n > 0).ok_or_else(not_found)
}

/// The report row and its parsed form, or 404/503/500.
fn load(state: &AppState, id: i64) -> Result<(&Db, Report), Resp> {
    let db = db(state)?;
    let row = db.report_get(id).map_err(|_| internal("database read failed"))?.ok_or_else(not_found)?;
    let rep = Report::from_row(&row).map_err(|e| internal(&format!("stored report is damaged: {e}")))?;
    Ok((db, rep))
}

/// Run `f` with the database, the loaded cache and a model built from the current rules and aliases.
fn with_data<T>(state: &AppState, f: impl FnOnce(&Db, &Cache, &Model) -> T) -> Result<T, Resp> {
    let db = db(state)?;
    let guard = state.read_cache();
    let Some(cache) = guard.as_ref() else {
        return Err(envelope(503, "unavailable", "cache not loaded"));
    };
    let aliases = db.payee_aliases().map_err(|_| internal("database read failed"))?;
    let rules = db.list_rules().map_err(|_| internal("database read failed"))?;
    let model = Model::build(cache, &aliases, &rules);
    Ok(f(db, cache, &model))
}

fn opts(state: &AppState) -> Options {
    Options { exclude_transfers: mailglue::exclude_transfers(&state.settings) }
}

fn allowlist(state: &AppState) -> Vec<String> {
    mailglue::allowlist(&state.settings)
}

/// Optional string field: absent or null is `None`; a non-string is an error.
fn str_field<'a>(m: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, Resp> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(invalid(&format!("{key} must be a string"))),
    }
}

/// A validated report, ready to store.
struct Validated {
    name: String,
    query: String,
    shape: Shape,
    period: Period,
    schedule: Schedule,
    recipients: Vec<String>,
    redact: bool,
}

impl Validated {
    fn fields(&self) -> ReportFields {
        ReportFields {
            name: self.name.clone(),
            query: self.query.clone(),
            shape: self.shape.name().to_string(),
            period: self.period.name().to_string(),
            schedule_json: self.schedule.to_json().to_string(),
            recipients_json: serde_json::to_string(&self.recipients).unwrap_or_else(|_| "[]".to_string()),
            redact: self.redact,
        }
    }
}

fn recipient_error(msg: String) -> Resp {
    if msg.starts_with("recipient not allowed") {
        envelope(400, "recipient_not_allowed", &msg)
    } else {
        invalid(&msg)
    }
}

/// Validate the writable fields. `base` supplies the value of any field the body leaves out (update).
fn validate(state: &AppState, m: &Map<String, Value>, base: Option<&Report>, today: NaiveDate) -> Result<Validated, Resp> {
    let name = match str_field(m, "name")? {
        Some(s) => check_name(s).map_err(|e| invalid(&e))?,
        None => match base {
            Some(b) => b.name.clone(),
            None => return Err(invalid("name is required")),
        },
    };

    let query = match str_field(m, "query")? {
        Some(q) => q.to_string(),
        None => base.map(|b| b.query.clone()).unwrap_or_default(),
    };
    if let Err(e) = model::check_query(&query, today) {
        return Err(parse_error(&e));
    }

    let shape = match str_field(m, "shape")? {
        Some(s) => Shape::from_name(s)
            .ok_or_else(|| invalid(&format!("unknown shape {s:?} (table, by_category, by_payee, by_month, summary)")))?,
        None => base.map(|b| b.shape).ok_or_else(|| invalid("shape is required"))?,
    };

    let period = match str_field(m, "period")? {
        Some(s) => Period::from_name(s).ok_or_else(|| {
            invalid(&format!("unknown period {s:?} (last_7d, last_full_week, last_full_month, month_to_date, last_full_quarter, ytd, all)"))
        })?,
        None => base.map(|b| b.period).ok_or_else(|| invalid("period is required"))?,
    };

    let schedule = match m.get("schedule") {
        None | Some(Value::Null) => base.map(|b| b.schedule).ok_or_else(|| invalid("schedule is required"))?,
        Some(v) => Schedule::from_json(v).map_err(|e| envelope(400, "invalid_schedule", &e))?,
    };

    let allow = allowlist(state);
    let sender = state.reports.sender(&state.settings);
    let recipients = match m.get("recipients") {
        None | Some(Value::Null) => match base {
            Some(b) => b.recipients.clone(),
            None => model::check_recipients(None, &allow, sender.as_deref()).unwrap_or_default(),
        },
        Some(Value::Array(items)) => {
            let mut list = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    Value::String(s) => list.push(s.clone()),
                    _ => return Err(invalid("recipients must be a list of email addresses")),
                }
            }
            model::check_recipients(Some(list.as_slice()), &allow, sender.as_deref()).map_err(recipient_error)?
        }
        Some(_) => return Err(invalid("recipients must be a list of email addresses")),
    };

    let redact = match m.get("redact") {
        None | Some(Value::Null) => base.map(|b| b.redact).unwrap_or(false),
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(invalid("redact must be true or false")),
    };

    Ok(Validated { name, query, shape, period, schedule, recipients, redact })
}

fn report_json(rep: &Report, last: Option<crate::db::reports::RunRow>, now: NaiveDateTime) -> Value {
    json!({
        "id": rep.id,
        "name": rep.name,
        "query": rep.query,
        "shape": rep.shape.name(),
        "period": rep.period.name(),
        "schedule": rep.schedule.to_json(),
        "recipients": rep.recipients,
        "redact": rep.redact,
        "enabled": rep.enabled,
        "last_run": last.map(|r| json!({ "status": r.status, "finished_at": r.finished_at, "detail": r.detail })),
        "next_runs": rep.schedule.next_runs(now, NEXT_RUNS).iter().map(iso).collect::<Vec<_>>(),
    })
}

/// `GET /reports`
pub fn list(state: &AppState) -> Resp {
    let db = match db(state) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let rows = match db.reports_list() {
        Ok(r) => r,
        Err(_) => return internal("database read failed"),
    };
    let now = Local::now().naive_local();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let last = db.last_run(r.id).ok().flatten();
        match Report::from_row(&r) {
            Ok(rep) => out.push(report_json(&rep, last, now)),
            Err(_) => out.push(json!({ "id": r.id, "name": r.name, "error": "stored report is damaged" })),
        }
    }
    (200, json!({ "reports": out }))
}

/// `POST /reports`
pub fn create(state: &AppState, body: &[u8]) -> Resp {
    let m = match parse_body(body) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let db = match db(state) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let v = match validate(state, &m, None, state.today()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match db.report_name_taken(&v.name, None) {
        Ok(true) => return duplicate(),
        Ok(false) => {}
        Err(_) => return internal("database read failed"),
    }
    match db.report_insert(&v.fields()) {
        Ok(id) => (200, json!({ "id": id })),
        Err(e) => {
            if db.report_name_taken(&v.name, None).unwrap_or(false) {
                duplicate()
            } else {
                log::error!("report insert: {:#}", e);
                internal("could not save the report")
            }
        }
    }
}

/// `PUT /reports/{id}`
pub fn update(state: &AppState, id: i64, body: &[u8]) -> Resp {
    let m = match parse_body(body) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let (db, rep) = match load(state, id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let v = match validate(state, &m, Some(&rep), state.today()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match db.report_name_taken(&v.name, Some(id)) {
        Ok(true) => return duplicate(),
        Ok(false) => {}
        Err(_) => return internal("database read failed"),
    }
    match db.report_update(id, &v.fields()) {
        Ok(true) => {}
        Ok(false) => return not_found(),
        Err(_) => return internal("could not save the report"),
    }
    if rep.enabled {
        if let Err(e) = sync_all(db, &state.reports) {
            log::error!("report timers after update: {}", e);
            return envelope(500, "timers", "saved, but the systemd timers could not be updated (run: omafinances report timers sync)");
        }
    }
    (200, json!({ "id": id }))
}

/// `DELETE /reports/{id}`: removes the timer first, so a systemd failure keeps the report for a retry.
pub fn delete(state: &AppState, id: i64) -> Resp {
    let (db, _rep) = match load(state, id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Err(e) = units::remove_report(&state.reports.units_dir(), id, state.reports.systemd.as_ref()) {
        log::error!("report timer removal: {}", e);
        return envelope(500, "timers", "could not remove the systemd timer; the report was kept");
    }
    match db.report_delete(id) {
        Ok(true) => (200, json!({ "ok": true })),
        Ok(false) => not_found(),
        Err(_) => internal("could not delete the report"),
    }
}

/// `POST /reports/{id}/preview`: no send, no run log.
pub fn preview(state: &AppState, id: i64) -> Resp {
    let (_db, rep) = match load(state, id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let opts = opts(state);
    let today = state.today();
    let res = with_data(state, |_db, cache, model| {
        let c = run::compute(cache, model, &rep, today, &opts)?;
        let r = render::render(&rep, &c);
        Ok::<Value, ParseError>(json!({
            "subject": r.subject,
            "text": r.text,
            "html": r.html,
            "csv_name": r.csv_name,
            "csv_rows": r.csv_rows,
            "rows": c.txns.len(),
            "period": {
                "from": c.span.from.map(|d| d.format("%Y-%m-%d").to_string()),
                "to": c.span.to.map(|d| d.format("%Y-%m-%d").to_string()),
                "label": c.span.label,
            },
        }))
    });
    match res {
        Ok(Ok(v)) => (200, v),
        Ok(Err(e)) => parse_error(&e),
        Err(r) => r,
    }
}

/// `POST /reports/{id}/send-test`: sends to the authenticated sender only, and logs a `test` run.
pub fn send_test(state: &AppState, id: i64) -> Resp {
    let (_db, rep) = match load(state, id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Ok(mailer) = state.reports.mailer(&state.settings) else {
        return not_authenticated();
    };
    let Ok(sender) = mailer.sender_address() else {
        return not_authenticated();
    };
    let m: &dyn Mailer = &*mailer;
    let opts = opts(state);
    let allow = allowlist(state);
    let today = state.today();
    let now = Local::now().naive_local();
    let res = with_data(state, |db, cache, model| {
        let ctx = Ctx { db, cache, model, today, now, opts, allowlist: &allow };
        run::execute(&ctx, &rep, Some(m), "test", std::slice::from_ref(&sender))
    });
    match res {
        Ok(Ok(o)) => (200, json!({ "status": o.status, "detail": o.detail, "rows": o.rows, "message_id": o.message_id })),
        Ok(Err(e)) => {
            log::error!("send-test: {:#}", e);
            internal("could not record the run")
        }
        Err(r) => r,
    }
}

/// `POST /reports/{id}/enable` `{enabled}`: writes or removes the timer. Enabling needs Gmail and a recipient.
pub fn enable(state: &AppState, id: i64, body: &[u8]) -> Resp {
    let m = match parse_body(body) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let want = match m.get("enabled") {
        Some(Value::Bool(b)) => *b,
        _ => return invalid("enabled must be true or false"),
    };
    let (db, rep) = match load(state, id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    if want {
        if state.reports.mailer(&state.settings).is_err() {
            return not_authenticated();
        }
        if rep.recipients.is_empty() {
            return invalid("add at least one recipient before enabling");
        }
    }
    if let Err(_e) = db.report_set_enabled(id, want) {
        return internal("could not update the report");
    }
    if let Err(e) = sync_all(db, &state.reports) {
        log::error!("report timers after enable: {}", e);
        let _ = db.report_set_enabled(id, rep.enabled);
        return envelope(500, "timers", "could not update the systemd timer; the report was not changed");
    }
    (200, json!({ "id": id, "enabled": want }))
}

/// `GET /reports/{id}/runs?limit=`
pub fn runs(state: &AppState, id: i64, query: &str) -> Resp {
    let (db, _rep) = match load(state, id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let q = match qs_map(query, &["limit"]) {
        Ok(q) => q,
        Err(r) => return r,
    };
    let limit = match q.get("limit") {
        None => DEFAULT_LIMIT,
        Some(v) => match v.parse::<i64>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => return invalid(&format!("limit must be 1..{MAX_LIMIT}")),
        },
    };
    match db.runs_list(id, limit) {
        Ok(rows) => (200, json!({ "runs": rows })),
        Err(_) => internal("database read failed"),
    }
}

/// `POST /reports/{id}/{action}` and `GET /reports/{id}/runs`.
pub fn action(state: &AppState, id: &str, action: &str, query: &str, body: &[u8]) -> Resp {
    let id = match parse_id(id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    match action {
        "preview" => preview(state, id),
        "send-test" => send_test(state, id),
        "enable" => enable(state, id, body),
        "runs" => runs(state, id, query),
        _ => not_found(),
    }
}

/// `PUT`/`DELETE /reports/{id}`.
pub fn one(state: &AppState, id: &str, method: &str, body: &[u8]) -> Resp {
    let id = match parse_id(id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    if method == "PUT" {
        update(state, id, body)
    } else {
        delete(state, id)
    }
}

/// `GET /schedule/preview?kind=&weekday=&day=&time=`
pub fn schedule_preview(query: &str) -> Resp {
    let q = match qs_map(query, &["kind", "weekday", "day", "time"]) {
        Ok(q) => q,
        Err(r) => return r,
    };
    let Some(kind) = q.get("kind") else {
        return invalid("kind is required (daily, weekly, monthly, quarterly)");
    };
    let time = q.get("time").map(String::as_str).unwrap_or("");
    match Schedule::from_query(kind, q.get("weekday").map(String::as_str), q.get("day").map(String::as_str), time) {
        Ok(s) => {
            let now = Local::now().naive_local();
            (
                200,
                json!({
                    "next_runs": s.next_runs(now, NEXT_RUNS).iter().map(iso).collect::<Vec<_>>(),
                    "oncalendar": s.to_oncalendar(),
                    "schedule": s.to_json(),
                }),
            )
        }
        Err(e) => envelope(400, "invalid_schedule", &e),
    }
}

/// `GET /mail/status`
pub fn mail_status(state: &AppState) -> Resp {
    let s = state.reports.mail_status(&state.settings);
    (200, json!({ "authenticated": s.authenticated, "sender": s.sender, "client_configured": s.client_configured }))
}

/// `key=value` pairs, percent-decoded. Unknown keys, duplicates and bad escapes are 400.
fn qs_map(query: &str, known: &[&str]) -> Result<BTreeMap<String, String>, Resp> {
    let mut out = BTreeMap::new();
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if !known.contains(&k) {
            return Err(invalid(&format!("unknown parameter: {k}")));
        }
        let v = query::percent_decode(v).map_err(|_| invalid(&format!("invalid value for {k}")))?;
        if out.insert(k.to_string(), v).is_some() {
            return Err(invalid(&format!("duplicate parameter: {k}")));
        }
    }
    Ok(out)
}
