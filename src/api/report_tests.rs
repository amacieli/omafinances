//! API-level tests for the report routes (`docs/PHASE6_CONTRACT.md`), through the real handlers on a synthetic
//! encrypted database. Mail is a fake mailer; systemd is a recording fake writing into a temp units directory.

use super::*;
use crate::config::Settings;
use crate::db::Db;
use crate::mail::{Mailer, OutMail};
use crate::report::units::Systemd;
use crate::report::ReportHost;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---- fixtures ----------------------------------------------------------------------------------

struct FakeMailer {
    authed: bool,
    fail: bool,
    sent: Mutex<Vec<OutMail>>,
}

impl Mailer for FakeMailer {
    fn send(&self, mail: &OutMail) -> Result<String> {
        if self.fail {
            return Err(anyhow!("provider rejected: ACME Hardware Store 1234.56"));
        }
        self.sent.lock().unwrap().push(mail.clone());
        Ok("msg-api-1".to_string())
    }
    fn sender_address(&self) -> Result<String> {
        if self.authed {
            Ok("me@example.com".to_string())
        } else {
            Err(anyhow!("no token"))
        }
    }
}

#[derive(Default)]
struct RecSystemd(Mutex<Vec<String>>);

impl Systemd for RecSystemd {
    fn user(&self, args: &[&str]) -> Result<(), String> {
        self.0.lock().unwrap().push(args.join(" "));
        Ok(())
    }
}

fn temp_units(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omafinances-api-units-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Synthetic DB + loaded cache. `mailer` None means "no Gmail" (the real mailer path, which fails without a client).
fn state_with(mailer: Option<Arc<FakeMailer>>, tag: &str) -> (PathBuf, AppState, Arc<RecSystemd>, PathBuf) {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    drop(db);
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    let mut settings = Settings::default();
    settings.reports.allowed_recipients = vec!["accountant@example.com".to_string()];
    let mut state = AppState::new(settings, Ok(db), "test");
    assert!(state.refresh().is_ok());
    let units_dir = temp_units(tag);
    let sd = Arc::new(RecSystemd::default());
    let mut host = ReportHost {
        mailer: None,
        systemd: sd.clone(),
        units_dir: Some(units_dir.clone()),
        binary: Some(PathBuf::from("/opt/omafinances/bin/omafinances")),
    };
    if let Some(m) = mailer {
        host.mailer = Some(m);
    }
    state.reports = host;
    (dir, state, sd, units_dir)
}

fn authed() -> Arc<FakeMailer> {
    Arc::new(FakeMailer { authed: true, fail: false, sent: Mutex::new(vec![]) })
}

fn call(state: &AppState, method: &str, path: &str, query: &str, body: &Value) -> (u16, Value) {
    handle_body(state, method, path, query, body.to_string().as_bytes())
}

fn weekly() -> Value {
    json!({ "kind": "weekly", "weekday": "mon", "time": "07:30" })
}

fn create_body(name: &str) -> Value {
    json!({
        "name": name,
        "query": "",
        "shape": "by_category",
        "period": "all",
        "schedule": weekly(),
        "recipients": ["me@example.com"],
        "redact": false
    })
}

fn create(state: &AppState, name: &str) -> i64 {
    let (code, body) = call(state, "POST", "/reports", "", &create_body(name));
    assert_eq!(code, 200, "{body}");
    body["id"].as_i64().unwrap()
}

/// Without a Gmail sender the sender address is not a known recipient, so use the allowlisted one.
fn create_unauth(state: &AppState, name: &str) -> i64 {
    let mut b = create_body(name);
    b["recipients"] = json!(["accountant@example.com"]);
    let (code, body) = call(state, "POST", "/reports", "", &b);
    assert_eq!(code, 200, "{body}");
    body["id"].as_i64().unwrap()
}

fn error_code(body: &Value) -> String {
    body["error"].as_str().unwrap_or_default().to_string()
}

// ---- CRUD and validation -----------------------------------------------------------------------

#[test]
fn create_list_update_delete_round_trip() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "crud");
    let id = create(&state, "Groceries");

    let (code, body) = call(&state, "GET", "/reports", "", &Value::Null);
    assert_eq!(code, 200);
    let reps = body["reports"].as_array().unwrap();
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0]["id"], id);
    assert_eq!(reps[0]["name"], "Groceries");
    assert_eq!(reps[0]["shape"], "by_category");
    assert_eq!(reps[0]["enabled"], false);
    assert_eq!(reps[0]["last_run"], Value::Null);
    assert_eq!(reps[0]["next_runs"].as_array().unwrap().len(), 3);

    let (code, body) = call(&state, "PUT", &format!("/reports/{id}"), "", &json!({ "shape": "summary", "redact": true }));
    assert_eq!(code, 200, "{body}");
    let (_, body) = call(&state, "GET", "/reports", "", &Value::Null);
    assert_eq!(body["reports"][0]["shape"], "summary");
    assert_eq!(body["reports"][0]["redact"], true);
    assert_eq!(body["reports"][0]["name"], "Groceries", "omitted fields keep their stored value");

    let (code, _) = call(&state, "DELETE", &format!("/reports/{id}"), "", &Value::Null);
    assert_eq!(code, 200);
    let (_, body) = call(&state, "GET", "/reports", "", &Value::Null);
    assert!(body["reports"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn bad_query_is_400_with_parser_text() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "badq");
    let mut b = create_body("Bad");
    b["query"] = json!("amount:>>");
    let (code, body) = call(&state, "POST", "/reports", "", &b);
    assert_eq!(code, 400, "{body}");
    assert_eq!(body["error"], "parse");
    assert!(!body["message"].as_str().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn validation_errors_are_400_and_leave_nothing_behind() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "valid");

    let mut b = create_body("Sched");
    b["schedule"] = json!({ "kind": "hourly" });
    let (code, body) = call(&state, "POST", "/reports", "", &b);
    assert_eq!((code, error_code(&body).as_str()), (400, "invalid_schedule"));

    let mut b = create_body("Shape");
    b["shape"] = json!("pie");
    assert_eq!(call(&state, "POST", "/reports", "", &b).0, 400);

    let mut b = create_body("Period");
    b["period"] = json!("last_decade");
    assert_eq!(call(&state, "POST", "/reports", "", &b).0, 400);

    let mut b = create_body("Empty");
    b["name"] = json!("   ");
    assert_eq!(call(&state, "POST", "/reports", "", &b).0, 400);

    let mut b = create_body("Stranger");
    b["recipients"] = json!(["stranger@example.org"]);
    let (code, body) = call(&state, "POST", "/reports", "", &b);
    assert_eq!((code, error_code(&body).as_str()), (400, "recipient_not_allowed"));

    let (_, body) = call(&state, "GET", "/reports", "", &Value::Null);
    assert!(body["reports"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn duplicate_name_is_409_on_create_and_update() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "dup");
    create(&state, "Same");
    let (code, body) = call(&state, "POST", "/reports", "", &create_body("Same"));
    assert_eq!((code, error_code(&body).as_str()), (409, "duplicate_name"));

    let other = create(&state, "Other");
    let (code, _) = call(&state, "PUT", &format!("/reports/{other}"), "", &json!({ "name": "Same" }));
    assert_eq!(code, 409);
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn allowlisted_recipient_is_accepted_on_update() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "allow");
    let id = create(&state, "Shared");
    let (code, body) = call(
        &state,
        "PUT",
        &format!("/reports/{id}"),
        "",
        &json!({ "recipients": ["Accountant@Example.com"] }),
    );
    assert_eq!(code, 200, "{body}");
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn unknown_report_is_404() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "nf");
    assert_eq!(call(&state, "PUT", "/reports/999", "", &json!({ "name": "x" })).0, 404);
    assert_eq!(call(&state, "DELETE", "/reports/999", "", &Value::Null).0, 404);
    assert_eq!(call(&state, "POST", "/reports/999/preview", "", &Value::Null).0, 404);
    assert_eq!(call(&state, "POST", "/reports/abc/preview", "", &Value::Null).0, 404);
    let _ = std::fs::remove_dir_all(units);
}

// ---- preview -----------------------------------------------------------------------------------

#[test]
fn preview_returns_content_and_never_sends() {
    let m = authed();
    let (_d, state, _sd, units) = state_with(Some(m.clone()), "prev");
    let id = create(&state, "Preview me");
    let (code, body) = call(&state, "POST", &format!("/reports/{id}/preview"), "", &Value::Null);
    assert_eq!(code, 200, "{body}");
    for key in ["subject", "text", "html", "csv_rows", "rows", "period"] {
        assert!(body.get(key).is_some(), "missing {key}: {body}");
    }
    assert!(body["subject"].as_str().unwrap().contains("Preview me"));
    assert!(body["period"]["label"].is_string());
    assert!(m.sent.lock().unwrap().is_empty(), "preview must not send");

    let (_, runs) = call(&state, "GET", &format!("/reports/{id}/runs"), "", &Value::Null);
    assert!(runs["runs"].as_array().unwrap().is_empty(), "preview must not log a run");
    let _ = std::fs::remove_dir_all(units);
}

// ---- send-test and enable without a token -----------------------------------------------------

#[test]
fn send_test_and_enable_need_gmail() {
    let (_d, state, sd, units) = state_with(None, "noauth");
    let id = create_unauth(&state, "Needs auth");
    let (code, body) = call(&state, "POST", &format!("/reports/{id}/send-test"), "", &Value::Null);
    assert_eq!((code, error_code(&body).as_str()), (409, "not_authenticated"));

    let (code, body) = call(&state, "POST", &format!("/reports/{id}/enable"), "", &json!({ "enabled": true }));
    assert_eq!((code, error_code(&body).as_str()), (409, "not_authenticated"));

    // Nothing was written or run.
    assert!(sd.0.lock().unwrap().is_empty());
    assert!(!units.join(format!("omafinances-report-{id}.timer")).exists());
    let (_, runs) = call(&state, "GET", &format!("/reports/{id}/runs"), "", &Value::Null);
    assert!(runs["runs"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn send_test_with_unauthenticated_fake_mailer_is_409() {
    let m = Arc::new(FakeMailer { authed: false, fail: false, sent: Mutex::new(vec![]) });
    let (_d, state, _sd, units) = state_with(Some(m.clone()), "unauthfake");
    let id = create_unauth(&state, "Revoked");
    let (code, body) = call(&state, "POST", &format!("/reports/{id}/send-test"), "", &Value::Null);
    assert_eq!((code, error_code(&body).as_str()), (409, "not_authenticated"));
    assert!(m.sent.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn send_test_ok_logs_a_test_run_and_sends_to_sender_only() {
    let m = authed();
    let (_d, state, _sd, units) = state_with(Some(m.clone()), "sendok");
    let id = create(&state, "Test ok");
    let (code, body) = call(&state, "POST", &format!("/reports/{id}/send-test"), "", &Value::Null);
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["status"], "ok");
    let sent = m.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, vec!["me@example.com".to_string()]);
    drop(sent);

    let (code, runs) = call(&state, "GET", &format!("/reports/{id}/runs"), "limit=5", &Value::Null);
    assert_eq!(code, 200);
    let runs = runs["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["trigger"], "test");
    assert_eq!(runs[0]["status"], "ok");
    assert_eq!(runs[0]["message_id"], "msg-api-1");
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn send_test_failure_logs_failed_without_row_text() {
    let m = Arc::new(FakeMailer { authed: true, fail: true, sent: Mutex::new(vec![]) });
    let (_d, state, _sd, units) = state_with(Some(m), "sendfail");
    let id = create(&state, "Test fail");
    let (code, _) = call(&state, "POST", &format!("/reports/{id}/send-test"), "", &Value::Null);
    assert_eq!(code, 200);
    let (_, runs) = call(&state, "GET", &format!("/reports/{id}/runs"), "", &Value::Null);
    let runs = runs["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["trigger"], "test");
    assert_eq!(runs[0]["status"], "failed");
    let all = runs[0].to_string();
    assert!(!all.contains("ACME") && !all.contains("1234.56"), "{all}");
    let _ = std::fs::remove_dir_all(units);
}

// ---- enable / disable / delete write and remove units ------------------------------------------

#[test]
fn enable_writes_units_and_disable_or_delete_removes_them() {
    let (_d, state, sd, units) = state_with(Some(authed()), "timers");
    let id = create(&state, "Timed");
    let timer = units.join(format!("omafinances-report-{id}.timer"));
    let service = units.join(format!("omafinances-report-{id}.service"));

    let (code, body) = call(&state, "POST", &format!("/reports/{id}/enable"), "", &json!({ "enabled": true }));
    assert_eq!(code, 200, "{body}");
    assert!(timer.exists() && service.exists());
    let svc = std::fs::read_to_string(&service).unwrap();
    assert!(svc.contains(&format!("report run {id} --trigger timer")), "{svc}");
    let t = std::fs::read_to_string(&timer).unwrap();
    assert!(t.contains("OnCalendar=Mon *-*-* 07:30:00"), "{t}");
    assert!(sd.0.lock().unwrap().iter().any(|c| c.contains("enable")));

    let (_, list) = call(&state, "GET", "/reports", "", &Value::Null);
    assert_eq!(list["reports"][0]["enabled"], true);

    let (code, _) = call(&state, "POST", &format!("/reports/{id}/enable"), "", &json!({ "enabled": false }));
    assert_eq!(code, 200);
    assert!(!timer.exists() && !service.exists(), "disable must remove the units");

    let (code, _) = call(&state, "POST", &format!("/reports/{id}/enable"), "", &json!({ "enabled": true }));
    assert_eq!(code, 200);
    assert!(timer.exists());
    let (code, _) = call(&state, "DELETE", &format!("/reports/{id}"), "", &Value::Null);
    assert_eq!(code, 200);
    assert!(!timer.exists() && !service.exists(), "delete must remove the units");
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn enable_body_must_be_boolean_and_recipients_required() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "enbody");
    let id = create(&state, "Body");
    let (code, _) = call(&state, "POST", &format!("/reports/{id}/enable"), "", &json!({ "enabled": "yes" }));
    assert_eq!(code, 400);
    let _ = std::fs::remove_dir_all(units);
}

// ---- runs ----------------------------------------------------------------------------------------

#[test]
fn runs_limit_is_validated() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "runs");
    let id = create(&state, "Runs");
    assert_eq!(call(&state, "GET", &format!("/reports/{id}/runs"), "limit=0", &Value::Null).0, 400);
    assert_eq!(call(&state, "GET", &format!("/reports/{id}/runs"), "limit=201", &Value::Null).0, 400);
    assert_eq!(call(&state, "GET", &format!("/reports/{id}/runs"), "limit=x", &Value::Null).0, 400);
    assert_eq!(call(&state, "GET", &format!("/reports/{id}/runs"), "bogus=1", &Value::Null).0, 400);
    assert_eq!(call(&state, "GET", &format!("/reports/{id}/runs"), "limit=200", &Value::Null).0, 200);
    let _ = std::fs::remove_dir_all(units);
}

// ---- schedule preview and mail status -----------------------------------------------------------

#[test]
fn schedule_preview_for_all_four_kinds() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "sched");
    let cases = [
        ("kind=daily&time=07:30", "*-*-* 07:30:00"),
        ("kind=weekly&weekday=mon&time=07:30", "Mon *-*-* 07:30:00"),
        ("kind=monthly&day=1&time=07:30", "*-*-01 07:30:00"),
        ("kind=quarterly&day=1&time=07:30", "*-01,04,07,10-01 07:30:00"),
    ];
    for (qs, oncal) in cases {
        let (code, body) = call(&state, "GET", "/schedule/preview", qs, &Value::Null);
        assert_eq!(code, 200, "{qs}: {body}");
        assert_eq!(body["next_runs"].as_array().unwrap().len(), 3, "{qs}");
        assert_eq!(body["oncalendar"], oncal, "{qs}");
    }
    let (code, body) = call(&state, "GET", "/schedule/preview", "kind=monthly&day=last&time=07:30", &Value::Null);
    assert_eq!(code, 200, "{body}");
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn schedule_preview_bad_inputs_are_400() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "schedbad");
    let bad = [
        "",
        "kind=hourly&time=07:30",
        "kind=weekly&weekday=funday&time=07:30",
        "kind=daily&time=25:00",
        "kind=daily&time=7:30",
        "kind=monthly&day=29&time=07:30",
        "kind=daily&time=07:30&extra=1",
        "kind=daily&time=07:30&time=08:00",
    ];
    for qs in bad {
        let (code, body) = call(&state, "GET", "/schedule/preview", qs, &Value::Null);
        assert_eq!(code, 400, "{qs}: {body}");
    }
    let _ = std::fs::remove_dir_all(units);
}

#[test]
fn mail_status_reports_fake_sender() {
    let (_d, state, _sd, units) = state_with(Some(authed()), "mailst");
    let (code, body) = call(&state, "GET", "/mail/status", "", &Value::Null);
    assert_eq!(code, 200);
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["sender"], "me@example.com");
    assert!(body["client_configured"].is_boolean());
    assert!(!body.to_string().contains("token"), "no token material in the response");
    let _ = std::fs::remove_dir_all(units);

    let m = Arc::new(FakeMailer { authed: false, fail: false, sent: Mutex::new(vec![]) });
    let (_d2, state2, _sd2, units2) = state_with(Some(m), "mailst2");
    let (code, body) = call(&state2, "GET", "/mail/status", "", &Value::Null);
    assert_eq!(code, 200);
    assert_eq!(body["authenticated"], false);
    assert!(body["sender"].is_null());
    let _ = std::fs::remove_dir_all(units2);
}
