//! Headless `omafinances report …` commands. No UI, no quickshell.
//!
//!   report run <id> [--trigger timer|catchup|manual]   exit 0 ok / 1 failed (fixed message on stderr, never row text)
//!   report timers sync                                 make systemd user units match the enabled reports
//!   report list

use crate::config;
use crate::db::Db;
use crate::report::{self, model::Report, run, ReportHost};
use crate::secrets;
use anyhow::{bail, Context, Result};
use chrono::Local;
use std::path::Path;

const USAGE: &str = "usage: report run <id> [--trigger timer|catchup|manual] | report timers sync | report list";

/// Entry point for `omafinances report <args…>`. Returns the process exit code.
pub fn run_cli(args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("timers") => match args.get(1).map(String::as_str) {
            Some("sync") if args.len() == 2 => cmd_timers_sync(),
            _ => bail!("{USAGE}"),
        },
        Some("list") if args.len() == 1 => cmd_list(),
        _ => bail!("{USAGE}"),
    }
}

fn open_db(settings: &config::Settings) -> Result<Db> {
    let pass = match secrets::db_passphrase() {
        Ok(Some(p)) => p,
        Ok(None) => bail!("database passphrase not in the keyring (run `omafinances keyring-import`)"),
        Err(e) => return Err(e.context("keyring")),
    };
    let db = Db::open(Path::new(&settings.database.path), &pass)?;
    db.migrate().context("migration failed")?;
    Ok(db)
}

fn load_settings() -> Result<config::Settings> {
    config::load_or_init(&config::config_dir()?)
}

fn cmd_run(args: &[String]) -> Result<i32> {
    let mut id: Option<i64> = None;
    let mut trigger = "manual".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--trigger" => {
                let v = args.get(i + 1).context("--trigger needs a value")?;
                if !matches!(v.as_str(), "timer" | "catchup" | "manual") {
                    bail!("--trigger must be timer, catchup or manual");
                }
                trigger = v.clone();
                i += 2;
            }
            other => {
                if id.is_some() {
                    bail!("unexpected argument {other:?}");
                }
                id = Some(other.parse().with_context(|| format!("report id must be a number, got {other:?}"))?);
                i += 1;
            }
        }
    }
    let id = id.context(USAGE)?;

    let settings = load_settings()?;
    let db = open_db(&settings)?;
    let row = db.report_get(id)?.with_context(|| format!("no report with id {id}"))?;
    let rep = Report::from_row(&row).map_err(|e| anyhow::anyhow!(e))?;

    let host = ReportHost::default();
    let mailer = host.mailer(&settings).ok();
    let sender = match &mailer {
        Some(m) => m.sender_address().ok(),
        None => None,
    };
    let to: Vec<String> = if rep.recipients.is_empty() {
        sender.clone().into_iter().collect()
    } else {
        rep.recipients.clone()
    };

    let (cache, model) = report::load_model(&db)?;
    let today = Local::now().date_naive();
    let now = Local::now().naive_local();
    let allow = report::mailglue::allowlist(&settings);
    let opts = run::Options { exclude_transfers: report::mailglue::exclude_transfers(&settings) };
    let ctx = run::Ctx { db: &db, cache: &cache, model: &model, today, now, opts, allowlist: &allow };
    let out = run::execute(&ctx, &rep, mailer.as_deref(), &trigger, &to)?;

    if out.status == "sent" || out.status == "ok" || out.status == "skipped" {
        println!("report {id}: {}", out.status);
        Ok(0)
    } else {
        eprintln!("report {id}: {} ({})", out.status, out.detail.as_deref().unwrap_or("no detail"));
        Ok(1)
    }
}

fn cmd_timers_sync() -> Result<i32> {
    let settings = load_settings()?;
    let db = open_db(&settings)?;
    let host = ReportHost::default();
    let r = report::sync_all(&db, &host).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "timers: written {:?}, removed {:?}, enabled {:?}",
        r.written, r.removed, r.enabled
    );
    Ok(0)
}

fn cmd_list() -> Result<i32> {
    let settings = load_settings()?;
    let db = open_db(&settings)?;
    for row in db.reports_list()? {
        let line = match Report::from_row(&row) {
            Ok(r) => format!("{}\t{}\t{}\t{}", r.id, if r.enabled { "on" } else { "off" }, r.shape.name(), r.name),
            Err(_) => format!("{}\tdamaged row", row.id),
        };
        println!("{line}");
    }
    Ok(0)
}
