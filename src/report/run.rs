//! Computing a report: parse the query, evaluate it against the cache, apply the period and transfer exclusion, then
//! shape the rows. Everything is display-safe on the way out (redaction is applied here, not in the renderer).
//!
//! Sign convention everywhere: a positive amount is money out, a negative amount is money in. `out` sums the
//! positive amounts, `inn` sums the negative ones (so it is <= 0), and `net = out + inn` (positive = net spend).

use super::model::{recipient_allowed, Report, Shape, Span};
use super::render;
use crate::cache::{Cache, Txn};
use crate::db::reports::NewRun;
use crate::db::Db;
use crate::insight::{anomalies, date_of, recurring, Model};
use crate::mail::Mailer;
use crate::search::{self, ParseError};
use chrono::{Days, NaiveDate, NaiveDateTime};
use std::collections::BTreeMap;

pub const TABLE_HTML_CAP: usize = 500;
pub const TOP_N: usize = 5;
pub const UPCOMING_DAYS: u64 = 14;
const REDACT_PAYEE_CHARS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Drop one side of each detected transfer pair (default true; `[reports] exclude_transfers`).
    pub exclude_transfers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Totals {
    pub count: usize,
    pub out: f64,
    pub inn: f64,
    pub net: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TxnLine {
    pub date: String,
    pub payee: String,
    pub account: String,
    pub category: String,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupLine {
    pub name: String,
    pub count: usize,
    pub out: f64,
    pub inn: f64,
    pub net: f64,
    /// Share of the period's money out, 0..=100.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnomalyLine {
    pub kind: &'static str,
    pub date: String,
    pub payee: String,
    pub amount: f64,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpcomingLine {
    pub payee: String,
    pub period: &'static str,
    pub amount: f64,
    pub next_expected: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub prev_span: Option<Span>,
    pub prev: Option<Totals>,
    pub delta: Option<f64>,
    /// Relative to |previous net|; `None` when the previous net is zero or there is no previous period.
    pub delta_pct: Option<f64>,
    pub top_categories: Vec<GroupLine>,
    pub top_payees: Vec<GroupLine>,
    pub anomalies: Vec<AnomalyLine>,
    pub upcoming: Vec<UpcomingLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Computed {
    pub shape: Shape,
    pub span: Span,
    pub redact: bool,
    pub totals: Totals,
    /// Distinct currency codes in the period (more than one means the totals are not converted).
    pub currencies: Vec<String>,
    /// Matched transactions, newest first. The table, the summary CSV and the row count come from here.
    pub txns: Vec<TxnLine>,
    /// Grouped shapes only (`by_*`), sorted as the shape requires.
    pub groups: Vec<GroupLine>,
    pub summary: Option<Summary>,
}

/// Compute the report for `today`. A query that does not parse is an error carrying the parser's message.
pub fn compute(cache: &Cache, model: &Model, rep: &Report, today: NaiveDate, opts: &Options) -> Result<Computed, ParseError> {
    let search = search::parse(&rep.query, today)?;
    let compiled = search.compile(cache);
    let span = rep.period.resolve(today);
    let selected = |i: usize| !(opts.exclude_transfers && model.transfers.is_transfer[i]) && compiled.matches(&cache.txns[i]);

    let mut idx: Vec<usize> = (0..cache.txns.len()).filter(|&i| selected(i) && in_span(&span, date_of(&cache.txns[i]))).collect();
    idx.sort_by_key(|&i| std::cmp::Reverse((cache.txns[i].date_key, cache.txns[i].id)));

    let totals = totals_of(cache, &idx);
    let mut currencies: Vec<String> = idx.iter().map(|&i| cache.txns[i].currency.clone()).filter(|c| !c.is_empty()).collect();
    currencies.sort();
    currencies.dedup();

    let txns: Vec<TxnLine> = idx.iter().map(|&i| txn_line(cache, &cache.txns[i], rep.redact)).collect();
    let groups = match rep.shape {
        Shape::ByCategory | Shape::ByPayee | Shape::ByMonth => groups_of(cache, &idx, rep.shape, rep.redact, totals.out),
        _ => Vec::new(),
    };

    let summary = if rep.shape == Shape::Summary {
        Some(summary_of(cache, model, rep, today, opts, &span, &idx, totals))
    } else {
        None
    };

    Ok(Computed { shape: rep.shape, span, redact: rep.redact, totals, currencies, txns, groups, summary })
}

#[allow(clippy::too_many_arguments)]
fn summary_of(
    cache: &Cache,
    model: &Model,
    rep: &Report,
    today: NaiveDate,
    opts: &Options,
    span: &Span,
    idx: &[usize],
    totals: Totals,
) -> Summary {
    let search = search::parse(&rep.query, today).ok();
    let compiled = search.as_ref().map(|s| s.compile(cache));
    let matches = |i: usize| compiled.as_ref().is_none_or(|c| c.matches(&cache.txns[i]));
    let selected = |i: usize| !(opts.exclude_transfers && model.transfers.is_transfer[i]) && matches(i);

    let prev_span = span.previous();
    let prev = prev_span.as_ref().map(|ps| {
        let pidx: Vec<usize> = (0..cache.txns.len()).filter(|&i| selected(i) && in_span(ps, date_of(&cache.txns[i]))).collect();
        totals_of(cache, &pidx)
    });
    let delta = prev.map(|p| totals.net - p.net);
    let delta_pct = match (prev, delta) {
        (Some(p), Some(d)) if p.net.abs() > 0.005 => Some(d / p.net.abs() * 100.0),
        _ => None,
    };

    let top = |shape: Shape| -> Vec<GroupLine> {
        let mut g = groups_of(cache, idx, shape, rep.redact, totals.out);
        g.truncate(TOP_N);
        g
    };

    let anomalies: Vec<AnomalyLine> = anomalies::detect(cache, model, today)
        .into_iter()
        .filter(|it| selected(it.idx) && in_span(span, date_of(&cache.txns[it.idx])))
        .map(|it| {
            let t = &cache.txns[it.idx];
            AnomalyLine {
                kind: it.kind,
                date: t.date.clone(),
                payee: payee_display(&t.payee, rep.redact),
                amount: t.amount,
                detail: it.detail,
            }
        })
        .collect();

    let horizon = today + Days::new(UPCOMING_DAYS);
    let upcoming: Vec<UpcomingLine> = recurring::detect(cache, model, today)
        .into_iter()
        .filter(|s| !s.negative)
        .filter(|s| NaiveDate::parse_from_str(&s.next_expected, "%Y-%m-%d").is_ok_and(|d| d >= today && d <= horizon))
        .map(|s| UpcomingLine {
            payee: payee_display(&s.payee, rep.redact),
            period: s.period,
            amount: s.amount_median,
            next_expected: s.next_expected,
        })
        .collect();

    Summary {
        prev_span,
        prev,
        delta,
        delta_pct,
        top_categories: top(Shape::ByCategory),
        top_payees: top(Shape::ByPayee),
        anomalies,
        upcoming,
    }
}

/// A transaction with no date is only inside an unbounded period.
fn in_span(span: &Span, d: Option<NaiveDate>) -> bool {
    match d {
        Some(d) => span.contains(d),
        None => span.from.is_none() && span.to.is_none(),
    }
}

fn totals_of(cache: &Cache, idx: &[usize]) -> Totals {
    let mut t = Totals { count: idx.len(), ..Default::default() };
    for &i in idx {
        let a = cache.txns[i].amount;
        if a > 0.0 {
            t.out += a;
        } else if a < 0.0 {
            t.inn += a;
        }
    }
    t.net = t.out + t.inn;
    t
}

/// Display payee: `(no payee)` for empty, and the first 12 characters when redacting.
pub fn payee_display(p: &str, redact: bool) -> String {
    let base = if p.trim().is_empty() { "(no payee)" } else { p };
    if redact {
        base.chars().take(REDACT_PAYEE_CHARS).collect()
    } else {
        base.to_string()
    }
}

fn category_of(cache: &Cache, t: &Txn) -> String {
    cache.category_path(t).unwrap_or("Uncategorized").to_string()
}

fn txn_line(cache: &Cache, t: &Txn, redact: bool) -> TxnLine {
    TxnLine {
        date: t.date.clone(),
        payee: payee_display(&t.payee, redact),
        account: if redact { "(hidden)".to_string() } else { t.account.clone().unwrap_or_default() },
        category: category_of(cache, t),
        amount: t.amount,
    }
}

fn groups_of(cache: &Cache, idx: &[usize], shape: Shape, redact: bool, total_out: f64) -> Vec<GroupLine> {
    // (count, out, inn) per display key. Month keys are `YYYY-MM`; an undated row goes last.
    let mut acc: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
    for &i in idx {
        let t = &cache.txns[i];
        let key = match shape {
            Shape::ByCategory => category_of(cache, t),
            Shape::ByPayee => payee_display(&t.payee, redact),
            _ => {
                if t.date.len() >= 7 && t.date_key != 0 {
                    t.date[..7].to_string()
                } else {
                    "~(no date)".to_string()
                }
            }
        };
        let e = acc.entry(key).or_insert((0, 0.0, 0.0));
        e.0 += 1;
        if t.amount > 0.0 {
            e.1 += t.amount;
        } else if t.amount < 0.0 {
            e.2 += t.amount;
        }
    }
    let mut out: Vec<GroupLine> = acc
        .into_iter()
        .map(|(name, (count, o, n))| GroupLine {
            name: name.trim_start_matches('~').to_string(),
            count,
            out: o,
            inn: n,
            net: o + n,
            share: if total_out > 0.0 { o / total_out * 100.0 } else { 0.0 },
        })
        .collect();
    if shape == Shape::ByMonth {
        // Chronological; the undated bucket (`~` sorts last in BTreeMap) stays at the end.
        return out;
    }
    out.sort_by(|a, b| b.out.partial_cmp(&a.out).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Outcome of one run, as stored in `report_run`. `detail` is a fixed phrase, never row text.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub status: &'static str,
    pub detail: Option<String>,
    pub rows: Option<i64>,
    pub message_id: Option<String>,
}

/// Everything one run needs besides the report itself. `now` is the local wall clock, passed in so tests pin it.
pub struct Ctx<'a> {
    pub db: &'a Db,
    pub cache: &'a Cache,
    pub model: &'a Model,
    pub today: NaiveDate,
    pub now: NaiveDateTime,
    pub opts: Options,
    /// `[reports] allowed_recipients`, already lowercased by the caller or not (comparison is case-insensitive).
    pub allowlist: &'a [String],
}

/// Compute, render, send and record one run. Every outcome is written to `report_run`; the stored `detail` is one
/// of a fixed set of phrases, so neither row text nor provider error text reaches the log.
///
/// `mailer` is `None` when no Gmail token is available; the run is then recorded as failed.
/// `trigger`: `timer` | `catchup` (both honour enabled and the last-day check) | `manual` | `test`.
pub fn execute(ctx: &Ctx, rep: &Report, mailer: Option<&dyn Mailer>, trigger: &str, to: &[String]) -> anyhow::Result<Outcome> {
    let started = ctx.now.format("%Y-%m-%d %H:%M:%S").to_string();
    let out = run_inner(ctx, rep, mailer, trigger, to);
    let finished = chrono::Local::now().naive_local().format("%Y-%m-%d %H:%M:%S").to_string();
    ctx.db.run_insert(&NewRun {
        report_id: rep.id,
        started_at: started,
        finished_at: Some(finished),
        status: out.status.to_string(),
        detail: out.detail.clone(),
        rows: out.rows,
        message_id: out.message_id.clone(),
        trigger: trigger.to_string(),
    })?;
    Ok(out)
}

fn outcome(status: &'static str, detail: &str) -> Outcome {
    Outcome { status, detail: Some(detail.to_string()), rows: None, message_id: None }
}

fn run_inner(ctx: &Ctx, rep: &Report, mailer: Option<&dyn Mailer>, trigger: &str, to: &[String]) -> Outcome {
    if matches!(trigger, "timer" | "catchup") {
        if !rep.enabled {
            return outcome("skipped", "report is disabled");
        }
        if rep.schedule.needs_day_check() && !rep.schedule.fires_on(ctx.today) {
            return outcome("skipped", "not the last day of the month");
        }
    }
    let Some(mailer) = mailer else {
        return outcome("failed", "gmail is not authenticated");
    };
    let sender = match mailer.sender_address() {
        Ok(s) => s,
        Err(_) => return outcome("failed", "gmail is not authenticated"),
    };
    if to.is_empty() {
        return outcome("failed", "no recipients");
    }
    if to.iter().any(|r| !recipient_allowed(ctx.allowlist, Some(&sender), r)) {
        return outcome("failed", "recipient not allowed");
    }
    let computed = match compute(ctx.cache, ctx.model, rep, ctx.today, &ctx.opts) {
        Ok(c) => c,
        Err(_) => return outcome("failed", "query does not parse"),
    };
    let mail = render::mail(rep, &computed, to);
    let rows = computed.txns.len() as i64;
    match mailer.send(&mail) {
        Ok(id) => Outcome {
            status: "ok",
            detail: Some(format!("sent to {} recipient(s)", to.len())),
            rows: Some(rows),
            message_id: Some(id),
        },
        // The provider error text is deliberately dropped: it can echo the message body.
        Err(_) => outcome("failed", "send failed"),
    }
}
