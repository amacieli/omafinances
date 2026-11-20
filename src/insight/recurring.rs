//! Recurring series: payees that repeat on a weekly, biweekly, monthly, quarterly or yearly beat.

use super::{date_of, day, median, money, sort_f64, Model};
use crate::cache::Cache;
use chrono::{Months, NaiveDate};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// (name, nominal days, min gap, max gap, months to the next one)
const PERIODS: &[(&str, i32, i32, i32, u32)] = &[
    ("weekly", 7, 6, 8, 0),
    ("biweekly", 14, 12, 16, 0),
    ("monthly", 30, 28, 32, 1),
    ("quarterly", 91, 88, 95, 3),
    ("yearly", 365, 360, 370, 12),
];
const MIN_OCCURRENCES: usize = 3;
const MAX_CV: f64 = 0.25;

pub struct Series {
    pub payee: String,
    pub negative: bool,
    pub period: &'static str,
    pub period_days: i32,
    pub count: usize,
    pub amount_median: f64,
    pub amount_cv: f64,
    pub last_date: String,
    pub next_expected: String,
    pub active: bool,
    pub account: Option<String>,
    pub txn_ids: Vec<i64>,
}

pub fn detect(cache: &Cache, model: &Model, today: NaiveDate) -> Vec<Series> {
    let mut groups: BTreeMap<(&str, bool), Vec<usize>> = BTreeMap::new();
    for (i, t) in cache.txns.iter().enumerate() {
        let norm = model.norms[i].as_str();
        if norm.is_empty() || day(t).is_none() || t.amount == 0.0 {
            continue;
        }
        groups.entry((norm, t.amount < 0.0)).or_default().push(i);
    }
    let mut out = Vec::new();
    for ((norm, neg), mut rows) in groups {
        if rows.len() < MIN_OCCURRENCES {
            continue;
        }
        rows.sort_by_key(|&i| (cache.txns[i].date_key, cache.txns[i].id));
        let days: Vec<i32> = rows.iter().map(|&i| day(&cache.txns[i]).unwrap_or(0)).collect();
        let gaps: Vec<i32> = days.windows(2).map(|w| w[1] - w[0]).collect();
        let mut sg: Vec<f64> = gaps.iter().map(|&g| g as f64).collect();
        sort_f64(&mut sg);
        let med = (median(&sg) + 0.5).floor() as i32;
        let Some(&(name, nominal, lo, hi, months)) = PERIODS.iter().find(|p| med >= p.2 && med <= p.3) else { continue };
        let within = gaps.iter().filter(|&&g| g >= lo && g <= hi).count();
        if within * 10 < gaps.len() * 7 {
            continue;
        }
        let abs: Vec<f64> = rows.iter().map(|&i| cache.txns[i].amount.abs()).collect();
        let mean = abs.iter().sum::<f64>() / abs.len() as f64;
        if mean <= 0.0 {
            continue;
        }
        let var = abs.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / abs.len() as f64;
        let cv = var.sqrt() / mean;
        if cv > MAX_CV {
            continue;
        }
        let mut signed: Vec<f64> = rows.iter().map(|&i| cache.txns[i].amount).collect();
        sort_f64(&mut signed);
        let last = &cache.txns[*rows.last().unwrap()];
        let last_d = date_of(last).unwrap_or(today);
        let next = if months > 0 {
            last_d.checked_add_months(Months::new(months)).unwrap_or(last_d)
        } else {
            last_d + chrono::Days::new(nominal as u64)
        };
        let active = (today - last_d).num_days() as f64 <= 1.5 * nominal as f64;
        let mut by_acct: HashMap<&str, usize> = HashMap::new();
        for &i in &rows {
            if let Some(a) = cache.txns[i].account.as_deref() {
                *by_acct.entry(a).or_insert(0) += 1;
            }
        }
        let mut accts: Vec<(&str, usize)> = by_acct.into_iter().collect();
        accts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        out.push(Series {
            payee: norm.to_string(),
            negative: neg,
            period: name,
            period_days: nominal,
            count: rows.len(),
            amount_median: median(&signed),
            amount_cv: (cv * 10_000.0).round() / 10_000.0,
            last_date: last.date.clone(),
            next_expected: next.format("%Y-%m-%d").to_string(),
            active,
            account: accts.first().map(|a| a.0.to_string()),
            txn_ids: rows.iter().map(|&i| cache.txns[i].id).collect(),
        });
    }
    let annual = |s: &Series| s.amount_median.abs() * 365.0 / s.period_days as f64;
    out.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(annual(b).partial_cmp(&annual(a)).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.payee.cmp(&b.payee))
            .then(a.negative.cmp(&b.negative))
    });
    out
}

pub fn report(series: &[Series]) -> Value {
    let rows: Vec<Value> = series
        .iter()
        .map(|s| {
            json!({
                "payee": s.payee,
                "period_days": s.period_days,
                "period": s.period,
                "count": s.count,
                "amount_median": money(s.amount_median),
                "amount_cv": s.amount_cv,
                "last_date": s.last_date,
                "next_expected": s.next_expected,
                "active": s.active,
                "account": s.account,
                "txn_ids": s.txn_ids,
            })
        })
        .collect();
    json!({ "series": rows })
}
