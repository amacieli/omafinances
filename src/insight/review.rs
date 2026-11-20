//! The review queue: one entry per payee group of uncategorized rows. A group is the normalized payee plus the sign of
//! the amount (money out vs money in), so refunds and charges never merge. The row fields describe the group's most
//! recent row (the representative); its options are the ones shown. Suggest mode orders groups by the top option's
//! confidence; frequency mode (bootstrap) orders them by group size.

use super::suggest::{options, Opt};
use super::Model;
use crate::cache::Cache;
use std::cmp::Reverse;
use std::collections::HashMap;

pub const MAX_LIMIT: usize = 200;
/// Most uncategorized row ids reported per group (most recent first).
pub const GROUP_IDS_CAP: usize = 500;

/// One payee group of uncategorized rows. Row indices refer to `Cache::txns`.
pub struct Group {
    /// Normalized payee (`smart::normalize` with the user's aliases).
    pub key: String,
    /// Money in (negative amounts). Otherwise money out.
    pub income: bool,
    /// Every uncategorized row of the group.
    pub count: usize,
    /// Sum of the amounts, rounded to cents. Positive = out.
    pub total: f64,
    /// Uncategorized rows of the group, most recent first, capped at `GROUP_IDS_CAP`.
    pub idx: Vec<usize>,
    /// Oldest row of the group.
    pub oldest: usize,
}

pub struct Row {
    /// The representative: the group's most recent row.
    pub idx: usize,
    /// Uncategorized rows sharing the representative's normalized payee, whatever their sign.
    pub payee_count: usize,
    pub options: Vec<Opt>,
    pub group: Group,
}

pub struct Queue {
    pub suggest_mode: bool,
    /// Number of groups, not rows.
    pub remaining: usize,
    pub rows: Vec<Row>,
}

/// Most recent first: date, then id.
fn newest(cache: &Cache, i: usize) -> Reverse<(u32, i64)> {
    Reverse((cache.txns[i].date_key, cache.txns[i].id))
}

pub fn queue(cache: &Cache, model: &Model, limit: usize, offset: usize) -> Queue {
    let suggest_mode = model.suggest(cache).enabled;
    let uncat: Vec<usize> = (0..cache.txns.len()).filter(|&i| cache.txns[i].category_id.is_none()).collect();
    let mut freq: HashMap<&str, usize> = HashMap::new();
    let mut buckets: HashMap<(&str, bool), Vec<usize>> = HashMap::new();
    for &i in &uncat {
        let norm = model.norms[i].as_str();
        *freq.entry(norm).or_insert(0) += 1;
        buckets.entry((norm, cache.txns[i].amount < 0.0)).or_default().push(i);
    }

    let mut rows: Vec<Row> = buckets
        .into_iter()
        .map(|((key, income), mut idx)| {
            idx.sort_by_key(|&i| newest(cache, i));
            let rep = idx[0];
            let oldest = idx[idx.len() - 1];
            let count = idx.len();
            let total: f64 = idx.iter().map(|&i| cache.txns[i].amount).sum();
            let payee_count = freq[key];
            let opts = if suggest_mode { options(model, cache, rep, true) } else { Vec::new() };
            let group = Group {
                key: key.to_string(),
                income,
                count,
                total: super::money(total),
                idx: idx.iter().take(GROUP_IDS_CAP).copied().collect(),
                oldest,
            };
            Row { idx: rep, payee_count, options: opts, group }
        })
        .collect();

    let top = |r: &Row| r.options.first().map_or(0.0, |o| o.confidence);
    let tie = |a: &Row, b: &Row| {
        newest(cache, a.idx)
            .cmp(&newest(cache, b.idx))
            .then_with(|| a.group.key.cmp(&b.group.key))
            .then_with(|| a.group.income.cmp(&b.group.income))
    };
    if suggest_mode {
        rows.sort_by(|a, b| top(b).partial_cmp(&top(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| tie(a, b)));
    } else {
        rows.sort_by(|a, b| b.group.count.cmp(&a.group.count).then_with(|| tie(a, b)));
    }

    let remaining = rows.len();
    let mut rows: Vec<Row> = rows.into_iter().skip(offset).take(limit).collect();
    if !suggest_mode {
        for r in &mut rows {
            r.options = options(model, cache, r.idx, false);
        }
    }
    Queue { suggest_mode, remaining, rows }
}

pub fn report(cache: &Cache, q: &Queue) -> serde_json::Value {
    let rows: Vec<_> = q
        .rows
        .iter()
        .map(|r| {
            let t = &cache.txns[r.idx];
            let g = &r.group;
            serde_json::json!({
                "id": t.id,
                "date": t.date,
                "payee": t.payee,
                "amount": super::money(t.amount),
                "account": t.account,
                "payee_count": r.payee_count,
                "options": r.options,
                "group_key": g.key,
                "group_count": g.count,
                "group_total": g.total,
                "group_ids": g.idx.iter().take(GROUP_IDS_CAP).map(|&i| cache.txns[i].id).collect::<Vec<_>>(),
                "group_date_min": cache.txns[g.oldest].date,
                "group_date_max": cache.txns[g.idx[0]].date,
            })
        })
        .collect();
    serde_json::json!({ "mode": if q.suggest_mode { "suggest" } else { "frequency" }, "remaining": q.remaining, "rows": rows })
}
