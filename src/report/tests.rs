//! Report unit tests (see docs/PHASE6_CONTRACT.md): schedule, periods, golden numbers, CSV guard, redaction,
//! migration v3, units through a fake systemd.

use super::model::{check_name, check_recipients, recipient_allowed, Period};
use super::render::{csv_cell, guard_formula, money};
use super::run::payee_display;
use super::schedule::Schedule;
use super::units::{self, Desired, Systemd};
use crate::db::Db;
use chrono::{NaiveDate, NaiveDateTime};
use std::sync::Mutex;

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

#[test]
fn weekly_schedule_next_runs_and_oncalendar() {
    // 2026-11-20 is a Friday; the next Mondays at 07:30 are 11-23, 11-30, 12-07.
    let s = Schedule::from_query("weekly", Some("mon"), None, "07:30").unwrap();
    let runs = s.next_runs(dt("2026-11-20 10:00"), 3);
    assert_eq!(runs, vec![dt("2026-11-23 07:30"), dt("2026-11-30 07:30"), dt("2026-12-07 07:30")]);
    assert_eq!(s.to_oncalendar(), "Mon *-*-* 07:30:00");
    assert!(!s.needs_day_check());
}

#[test]
fn next_runs_is_strictly_after_the_reference_time() {
    let s = Schedule::from_query("weekly", Some("mon"), None, "07:30").unwrap();
    assert_eq!(s.next_runs(dt("2026-11-23 07:30"), 1), vec![dt("2026-11-30 07:30")]);
}

#[test]
fn daily_monthly_last_oncalendar_golden() {
    assert_eq!(Schedule::from_query("daily", None, None, "06:00").unwrap().to_oncalendar(), "*-*-* 06:00:00");
    assert_eq!(Schedule::from_query("monthly", None, Some("15"), "08:00").unwrap().to_oncalendar(), "*-*-15 08:00:00");
    let last = Schedule::from_query("monthly", None, Some("last"), "08:00").unwrap();
    assert_eq!(last.to_oncalendar(), "*-*-28,29,30,31 08:00:00");
    assert!(last.needs_day_check());
}

#[test]
fn schedule_rejects_bad_input() {
    assert!(Schedule::from_query("weekly", None, None, "07:30").is_err());
    assert!(Schedule::from_query("weekly", Some("funday"), None, "07:30").is_err());
    assert!(Schedule::from_query("daily", None, None, "25:00").is_err());
    assert!(Schedule::from_query("fortnightly", None, None, "07:30").is_err());
}

#[test]
fn period_resolution_golden() {
    let today = d("2026-11-20");
    let m = Period::LastFullMonth.resolve(today);
    assert_eq!((m.from, m.to), (Some(d("2026-10-01")), Some(d("2026-10-31"))));
    let w = Period::LastFullWeek.resolve(today);
    assert_eq!((w.from, w.to), (Some(d("2026-11-09")), Some(d("2026-11-15"))));
    let t = Period::Last7d.resolve(today);
    assert_eq!((t.from, t.to), (Some(d("2026-11-14")), Some(today)));
    let mtd = Period::MonthToDate.resolve(today);
    assert_eq!((mtd.from, mtd.to), (Some(d("2026-11-01")), Some(today)));
    let q = Period::LastFullQuarter.resolve(today);
    assert_eq!((q.from, q.to), (Some(d("2026-07-01")), Some(d("2026-09-30"))));
}

#[test]
fn money_formats_two_decimals() {
    assert!(money(1234.5).contains("1,234.50") || money(1234.5).contains("1234.50"));
    assert!(money(-20.0).contains("20.00"));
}

#[test]
fn csv_injection_guard() {
    assert_eq!(guard_formula("=SUM(A1)"), "'=SUM(A1)");
    assert_eq!(guard_formula("+1"), "'+1");
    assert_eq!(guard_formula("-5"), "'-5");
    assert_eq!(guard_formula("@cmd"), "'@cmd");
    assert_eq!(guard_formula("Groceries"), "Groceries");
    assert_eq!(csv_cell("a,b"), "\"a,b\"");
    assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv_cell("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
}

#[test]
fn payee_redaction_truncates_and_names_blank() {
    assert_eq!(payee_display("ACME Hardware Store Downtown", true), "ACME Hardwar");
    assert_eq!(payee_display("ACME Hardware Store Downtown", false), "ACME Hardware Store Downtown");
    assert_eq!(payee_display("   ", false), "(no payee)");
}

#[test]
fn recipients_and_names_are_validated() {
    let allow = vec!["friend@example.com".to_string()];
    assert!(recipient_allowed(&allow, Some("me@example.com"), "ME@example.com"));
    assert!(recipient_allowed(&allow, Some("me@example.com"), "friend@example.com"));
    assert!(!recipient_allowed(&allow, Some("me@example.com"), "stranger@example.com"));
    assert!(check_recipients(Some(&["stranger@example.com".to_string()]), &allow, Some("me@example.com")).is_err());
    assert!(check_name("   ").is_err());
    assert!(check_name("Monthly summary").is_ok());
}

struct FakeSystemd(Mutex<Vec<String>>);

impl Systemd for FakeSystemd {
    fn user(&self, args: &[&str]) -> Result<(), String> {
        self.0.lock().unwrap().push(args.join(" "));
        Ok(())
    }
}

#[test]
fn units_sync_writes_timers_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("omafinances-units-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sd = FakeSystemd(Mutex::new(vec![]));
    let sched = Schedule::from_query("weekly", Some("mon"), None, "07:30").unwrap();
    let bin = std::path::PathBuf::from("/opt/omafinances/bin/omafinances");
    let desired = vec![Desired { id: 7, schedule: sched }];

    let first = units::sync(&dir, &bin, &desired, &sd).unwrap();
    assert_eq!(first.written, vec![7]);
    let svc = std::fs::read_to_string(dir.join("omafinances-report-7.service")).unwrap();
    assert!(svc.contains("report run 7 --trigger timer"), "{svc}");
    assert!(svc.contains("Type=oneshot"));
    let timer = std::fs::read_to_string(dir.join("omafinances-report-7.timer")).unwrap();
    assert!(timer.contains("OnCalendar=Mon *-*-* 07:30:00"), "{timer}");
    assert!(timer.contains("WantedBy=timers.target"));

    let second = units::sync(&dir, &bin, &desired, &sd).unwrap();
    assert!(second.written.is_empty(), "unchanged units must not be rewritten");

    let gone = units::sync(&dir, &bin, &[], &sd).unwrap();
    assert_eq!(gone.removed, vec![7]);
    assert!(!dir.join("omafinances-report-7.timer").exists());
    assert!(sd.0.lock().unwrap().iter().any(|c| c.contains("daemon-reload")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// v3 uses IF NOT EXISTS: a database that is back at v2 with the tables present migrates again cleanly.
#[test]
fn migration_v3_is_rerunnable() {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    drop(db);
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.pragma_update(None, "key", crate::db::tests::PASS).unwrap();
        c.execute_batch("PRAGMA user_version=2;").unwrap();
    }
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    let rep = db.migrate().unwrap();
    assert_eq!((rep.from, rep.to), (2, 5));
    assert!(db.reports_list().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

// ---- end to end with a fake mailer ----------------------------------------------------------

use super::model::Report;
use super::run::{self, Ctx, Options};
use crate::mail::{Mailer, OutMail};
use anyhow::{anyhow, Result};

struct FakeMailer {
    sent: Mutex<Vec<OutMail>>,
    fail: bool,
}

impl Mailer for FakeMailer {
    fn send(&self, mail: &OutMail) -> Result<String> {
        if self.fail {
            // a provider error may echo request content: it must never reach the run row or the log
            return Err(anyhow!("provider rejected: ACME Hardware Store 1234.56"));
        }
        self.sent.lock().unwrap().push(OutMail {
            to: mail.to.clone(),
            subject: mail.subject.clone(),
            text: mail.text.clone(),
            html: mail.html.clone(),
            attachments: vec![],
        });
        Ok("msg-1".to_string())
    }
    fn sender_address(&self) -> Result<String> {
        Ok("me@example.com".to_string())
    }
}

fn fixture_report(db: &Db) -> Report {
    let sched = Schedule::from_query("weekly", Some("mon"), None, "07:30").unwrap();
    let id = db
        .report_insert(&crate::db::reports::ReportFields {
            name: "Monthly summary".to_string(),
            query: String::new(),
            shape: "summary".to_string(),
            period: "last_full_month".to_string(),
            schedule_json: sched.to_json().to_string(),
            recipients_json: "[\"me@example.com\"]".to_string(),
            redact: true,
        })
        .unwrap();
    db.report_set_enabled(id, true).unwrap();
    let row = db.report_get(id).unwrap().unwrap();
    Report::from_row(&row).unwrap()
}

fn run_once(db: &Db, rep: &Report, mailer: &dyn Mailer) -> run::Outcome {
    let (cache, model) = super::load_model(db).unwrap();
    let ctx = Ctx {
        db,
        cache: &cache,
        model: &model,
        today: d("2026-11-20"),
        now: dt("2026-11-20 07:30"),
        opts: Options { exclude_transfers: true },
        allowlist: &[],
    };
    run::execute(&ctx, rep, Some(mailer), "manual", &["me@example.com".to_string()]).unwrap()
}

#[test]
fn end_to_end_run_sends_and_records_the_row() {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    let rep = fixture_report(&db);
    let m = FakeMailer { sent: Mutex::new(vec![]), fail: false };
    let out = run_once(&db, &rep, &m);
    assert_eq!(out.status, "ok", "{:?}", out.detail);
    assert_eq!(out.message_id.as_deref(), Some("msg-1"));
    let sent = m.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, vec!["me@example.com".to_string()]);
    assert!(!sent[0].html.contains("http://") && !sent[0].html.contains("https://"), "no remote URLs in HTML");
    assert!(!sent[0].html.to_lowercase().contains("<script"));
    let runs = db.runs_list(rep.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "ok");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn failed_send_stores_no_row_text() {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    let rep = fixture_report(&db);
    let m = FakeMailer { sent: Mutex::new(vec![]), fail: true };
    let out = run_once(&db, &rep, &m);
    assert_eq!(out.status, "failed");
    assert!(m.sent.lock().unwrap().is_empty());
    let runs = db.runs_list(rep.id, 10).unwrap();
    assert_eq!(runs.len(), 1, "the failed attempt is still recorded");
    assert_eq!(runs[0].status, "failed");
    let stored = format!("{:?}", runs[0]);
    assert!(!stored.contains("ACME") && !stored.contains("1234.56"), "{stored}");
    let _ = std::fs::remove_dir_all(dir);
}

// ---- headless `report run` logic (the CLI's library path, with a fake mailer) ------------------

fn cli_ctx<'a>(db: &'a Db, cache: &'a crate::cache::Cache, model: &'a crate::insight::Model, allow: &'a [String]) -> run::Ctx<'a> {
    run::Ctx {
        db,
        cache,
        model,
        today: d("2026-11-20"),
        now: dt("2026-11-20 07:30"),
        opts: Options { exclude_transfers: true },
        allowlist: allow,
    }
}

#[test]
fn cli_run_ok_and_failed_map_to_exit_statuses() {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    let rep = fixture_report(&db);
    let (cache, model) = super::load_model(&db).unwrap();
    let allow: Vec<String> = vec![];
    let to = vec!["me@example.com".to_string()];

    // ok: status "ok" is exit 0 in cmd_run
    let good = FakeMailer { sent: Mutex::new(vec![]), fail: false };
    let out = run::execute(&cli_ctx(&db, &cache, &model, &allow), &rep, Some(&good), "manual", &to).unwrap();
    assert_eq!(out.status, "ok");

    // failed: exit 1; the detail is a fixed phrase, never provider text
    let bad = FakeMailer { sent: Mutex::new(vec![]), fail: true };
    let out = run::execute(&cli_ctx(&db, &cache, &model, &allow), &rep, Some(&bad), "manual", &to).unwrap();
    assert_eq!(out.status, "failed");
    assert_eq!(out.detail.as_deref(), Some("send failed"));
    assert!(!out.detail.unwrap().contains("ACME"));

    // no Gmail at all: failed, nothing sent
    let out = run::execute(&cli_ctx(&db, &cache, &model, &allow), &rep, None, "manual", &to).unwrap();
    assert_eq!(out.status, "failed");

    // a recipient outside the allowlist is refused even by a manual run
    let stranger = vec!["stranger@example.org".to_string()];
    let out = run::execute(&cli_ctx(&db, &cache, &model, &allow), &rep, Some(&good), "manual", &stranger).unwrap();
    assert_eq!(out.status, "failed");
    assert_eq!(out.detail.as_deref(), Some("recipient not allowed"));

    assert_eq!(good.sent.lock().unwrap().len(), 1, "only the ok run sent mail");
    assert_eq!(db.runs_list(rep.id, 10).unwrap().len(), 4, "every attempt is recorded");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_timer_run_on_disabled_report_is_skipped_but_manual_sends() {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    let rep = fixture_report(&db);
    db.report_set_enabled(rep.id, false).unwrap();
    let rep = Report::from_row(&db.report_get(rep.id).unwrap().unwrap()).unwrap();
    let (cache, model) = super::load_model(&db).unwrap();
    let allow: Vec<String> = vec![];
    let to = vec!["me@example.com".to_string()];
    let m = FakeMailer { sent: Mutex::new(vec![]), fail: false };

    let out = run::execute(&cli_ctx(&db, &cache, &model, &allow), &rep, Some(&m), "timer", &to).unwrap();
    assert_eq!(out.status, "skipped");
    assert!(m.sent.lock().unwrap().is_empty());

    let out = run::execute(&cli_ctx(&db, &cache, &model, &allow), &rep, Some(&m), "manual", &to).unwrap();
    assert_eq!(out.status, "ok");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_missing_report_is_not_found() {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    assert!(db.report_get(424242).unwrap().is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_dispatch_rejects_bad_arguments_before_touching_the_keyring() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    assert!(super::cli::run_cli(&s(&[])).is_err());
    assert!(super::cli::run_cli(&s(&["run"])).is_err());
    assert!(super::cli::run_cli(&s(&["run", "abc"])).is_err());
    assert!(super::cli::run_cli(&s(&["run", "1", "--trigger", "bogus"])).is_err());
    assert!(super::cli::run_cli(&s(&["timers"])).is_err());
    assert!(super::cli::run_cli(&s(&["list", "extra"])).is_err());
}
