//! Phase 5 intelligence: category suggestions, review queue, recurring series, anomalies, transfer pairing and the
//! dashboard. Everything is deterministic and pure over a `Cache`; the API (`api/insight.rs`) only parses and shapes.
//!
//! `Model` is the per-cache derived state (normalized payees, enabled rules, transfer pairs, and the lazily trained
//! suggestion model). `AppState` keeps one and drops it on every cache refresh.

pub mod anomalies;
pub mod dashboard;
pub mod recurring;
pub mod review;
pub mod suggest;
pub mod transfers;

#[cfg(test)]
mod tests;

use crate::cache::{Cache, Txn};
use crate::db::rules::RuleRow;
use crate::smart::normalize::{normalize, Aliases};
use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct Model {
    /// Normalized payee per transaction, indexed like `Cache::txns`.
    pub norms: Vec<String>,
    pub rules: Vec<suggest::CompiledRule>,
    pub transfers: transfers::Transfers,
    suggest: OnceLock<suggest::Suggest>,
}

impl Model {
    pub fn build(cache: &Cache, aliases: &Aliases, rules: &[RuleRow]) -> Model {
        let mut memo: HashMap<&str, usize> = HashMap::new();
        let mut distinct: Vec<String> = Vec::new();
        let mut at: Vec<usize> = Vec::with_capacity(cache.txns.len());
        for t in &cache.txns {
            let i = *memo.entry(t.payee.as_str()).or_insert_with(|| {
                distinct.push(normalize(&t.payee, aliases));
                distinct.len() - 1
            });
            at.push(i);
        }
        let norms: Vec<String> = at.iter().map(|&i| distinct[i].clone()).collect();
        let transfers = transfers::detect(cache);
        Model { norms, rules: suggest::compile_rules(cache, rules), transfers, suggest: OnceLock::new() }
    }

    /// The trained suggestion model (trained and cross-validated on first use).
    pub fn suggest(&self, cache: &Cache) -> &suggest::Suggest {
        self.suggest.get_or_init(|| suggest::Suggest::train(cache, &self.norms))
    }
}

/// Day number (days from the common era) of a transaction, `None` when the stored date is not a calendar date.
pub(crate) fn day(t: &Txn) -> Option<i32> {
    date_of(t).map(|d| d.num_days_from_ce_compat())
}

pub(crate) fn date_of(t: &Txn) -> Option<NaiveDate> {
    if t.date_key == 0 {
        return None;
    }
    NaiveDate::from_ymd_opt((t.date_key / 10000) as i32, (t.date_key / 100 % 100) as u32, t.date_key % 100)
}

trait Ce {
    fn num_days_from_ce_compat(&self) -> i32;
}
impl Ce for NaiveDate {
    fn num_days_from_ce_compat(&self) -> i32 {
        use chrono::Datelike;
        self.num_days_from_ce()
    }
}

pub(crate) fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

pub(crate) fn sort_f64(v: &mut [f64]) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

pub(crate) fn money(x: f64) -> f64 {
    crate::smart::money(x)
}
