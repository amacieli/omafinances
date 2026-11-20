//! Transfer pairing: an outflow and an inflow of the same amount in different accounts within four days.
//! One-to-one, greedy by smallest day gap, then lowest out id, then lowest in id.

use super::day;
use crate::cache::Cache;
use std::collections::HashMap;

pub const MAX_GAP_DAYS: i32 = 4;

#[derive(Debug, Clone, Copy)]
pub struct Pair {
    /// Indices into `Cache::txns`.
    pub out: usize,
    pub inn: usize,
    pub days: i32,
    pub amount: f64,
}

pub struct Transfers {
    /// Newest first (later row's date), ties by out id.
    pub pairs: Vec<Pair>,
    /// Per transaction index: is it one side of a pair.
    pub is_transfer: Vec<bool>,
}

pub fn detect(cache: &Cache) -> Transfers {
    type Side = Vec<(i32, usize)>;
    let mut buckets: HashMap<i64, (Side, Side)> = HashMap::new();
    for (i, t) in cache.txns.iter().enumerate() {
        let (Some(d), Some(_)) = (day(t), t.account_id) else { continue };
        let cents = (t.amount.abs() * 100.0).round() as i64;
        if cents == 0 {
            continue;
        }
        let e = buckets.entry(cents).or_default();
        if t.amount > 0.0 {
            e.0.push((d, i));
        } else {
            e.1.push((d, i));
        }
    }
    // (gap, out id, in id, out idx, in idx)
    let mut cands: Vec<(i32, i64, i64, usize, usize)> = Vec::new();
    for (outs, ins) in buckets.values_mut() {
        if outs.is_empty() || ins.is_empty() {
            continue;
        }
        ins.sort_unstable();
        for &(d, o) in outs.iter() {
            let start = ins.partition_point(|&(x, _)| x < d - MAX_GAP_DAYS);
            for &(x, n) in &ins[start..] {
                if x > d + MAX_GAP_DAYS {
                    break;
                }
                if cache.txns[o].account_id != cache.txns[n].account_id {
                    cands.push(((x - d).abs(), cache.txns[o].id, cache.txns[n].id, o, n));
                }
            }
        }
    }
    cands.sort_unstable();
    let mut is_transfer = vec![false; cache.txns.len()];
    let mut pairs = Vec::new();
    for (gap, _, _, o, n) in cands {
        if is_transfer[o] || is_transfer[n] {
            continue;
        }
        is_transfer[o] = true;
        is_transfer[n] = true;
        pairs.push(Pair { out: o, inn: n, days: gap, amount: cache.txns[o].amount });
    }
    pairs.sort_by(|a, b| {
        let la = cache.txns[a.out].date_key.max(cache.txns[a.inn].date_key);
        let lb = cache.txns[b.out].date_key.max(cache.txns[b.inn].date_key);
        lb.cmp(&la).then(cache.txns[a.out].id.cmp(&cache.txns[b.out].id))
    });
    Transfers { pairs, is_transfer }
}

pub fn report(cache: &Cache, tr: &Transfers) -> serde_json::Value {
    let total: f64 = tr.pairs.iter().map(|p| p.amount).sum();
    let name = |i: usize| cache.txns[i].account.clone();
    let pairs: Vec<_> = tr
        .pairs
        .iter()
        .map(|p| {
            serde_json::json!({
                "out_id": cache.txns[p.out].id,
                "in_id": cache.txns[p.inn].id,
                "amount": super::money(p.amount),
                "days_apart": p.days,
                "out_account": name(p.out),
                "in_account": name(p.inn),
            })
        })
        .collect();
    serde_json::json!({ "pairs": pairs, "count": tr.pairs.len(), "total": super::money(total) })
}
