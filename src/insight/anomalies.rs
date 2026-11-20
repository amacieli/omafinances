//! Anomalies: amount spikes, duplicates and brand-new payees. Rows that are one side of a transfer pair are ignored.

use super::{day, median, money, sort_f64, Model};
use crate::cache::Cache;
use chrono::NaiveDate;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_ITEMS: usize = 200;
const MIN_PRIOR: usize = 5;
const NEW_PAYEE_DAYS: i64 = 30;
const NEW_PAYEE_MIN: f64 = 50.0;
const DUP_DAYS: i32 = 3;

pub struct Item {
    pub kind: &'static str,
    pub idx: usize,
    pub detail: String,
}

fn kind_rank(k: &str) -> u8 {
    match k {
        "amount_spike" => 0,
        "duplicate" => 1,
        _ => 2,
    }
}

pub fn detect(cache: &Cache, model: &Model, today: NaiveDate) -> Vec<Item> {
    let rows: Vec<usize> = (0..cache.txns.len())
        .filter(|&i| !model.transfers.is_transfer[i] && day(&cache.txns[i]).is_some() && !model.norms[i].is_empty())
        .collect();
    let key = |i: usize| (cache.txns[i].date_key, cache.txns[i].id);
    let mut items: Vec<Item> = Vec::new();

    // amount spikes, per (payee, sign), against the rows before it
    let mut groups: BTreeMap<(&str, bool), Vec<usize>> = BTreeMap::new();
    for &i in &rows {
        groups.entry((model.norms[i].as_str(), cache.txns[i].amount < 0.0)).or_default().push(i);
    }
    for g in groups.values_mut() {
        g.sort_by_key(|&i| key(i));
        let mut prior: Vec<f64> = Vec::new(); // sorted |amount|
        for &i in g.iter() {
            let a = cache.txns[i].amount.abs();
            if prior.len() >= MIN_PRIOR {
                let m = median(&prior);
                if m > 0.0 && a > m && a >= 2.0 * m {
                    let mut dev: Vec<f64> = prior.iter().map(|x| (x - m).abs()).collect();
                    sort_f64(&mut dev);
                    let sigma = 1.4826 * median(&dev);
                    if a > m + 3.0 * sigma {
                        items.push(Item {
                            kind: "amount_spike",
                            idx: i,
                            detail: format!("{:.1}x this payee's median of ${:.2}", a / m, m),
                        });
                    }
                }
            }
            let at = prior.partition_point(|&x| x < a);
            prior.insert(at, a);
        }
    }

    // duplicates: same payee, amount (to the cent) and account within three days
    let mut dups: BTreeMap<(&str, i64, Option<i64>), Vec<usize>> = BTreeMap::new();
    for &i in &rows {
        let t = &cache.txns[i];
        let cents = (t.amount * 100.0).round() as i64;
        if cents != 0 {
            dups.entry((model.norms[i].as_str(), cents, t.account_id)).or_default().push(i);
        }
    }
    for g in dups.values_mut() {
        g.sort_by_key(|&i| key(i));
        for (j, &i) in g.iter().enumerate() {
            let d = day(&cache.txns[i]).unwrap_or(0);
            let earlier = g[..j].iter().rev().find(|&&e| d - day(&cache.txns[e]).unwrap_or(0) <= DUP_DAYS);
            if let Some(&e) = earlier {
                let gap = d - day(&cache.txns[e]).unwrap_or(0);
                items.push(Item {
                    kind: "duplicate",
                    idx: i,
                    detail: format!("same payee, amount and account as #{} {} day(s) earlier", cache.txns[e].id, gap),
                });
            }
        }
    }

    // new payees: the first row of a payee, in the last 30 days, of at least $50
    let mut first: BTreeMap<&str, usize> = BTreeMap::new();
    for &i in &rows {
        let e = first.entry(model.norms[i].as_str()).or_insert(i);
        if key(i) < key(*e) {
            *e = i;
        }
    }
    let today_key = today.format("%Y%m%d").to_string().parse::<u32>().unwrap_or(0);
    let from_key = (today - chrono::Days::new(NEW_PAYEE_DAYS as u64)).format("%Y%m%d").to_string().parse::<u32>().unwrap_or(0);
    for &i in first.values() {
        let t = &cache.txns[i];
        if t.date_key >= from_key && t.date_key <= today_key && t.amount >= NEW_PAYEE_MIN {
            items.push(Item { kind: "new_payee", idx: i, detail: format!("first time seen, ${:.2}", t.amount) });
        }
    }

    items.sort_by(|a, b| {
        key(b.idx).cmp(&key(a.idx)).then(kind_rank(a.kind).cmp(&kind_rank(b.kind)))
    });
    items.truncate(MAX_ITEMS);
    items
}

pub fn report(cache: &Cache, items: &[Item]) -> Value {
    let rows: Vec<Value> = items
        .iter()
        .map(|it| {
            let t = &cache.txns[it.idx];
            json!({
                "kind": it.kind,
                "txn_id": t.id,
                "date": t.date,
                "payee": t.payee,
                "amount": money(t.amount),
                "detail": it.detail,
            })
        })
        .collect();
    json!({ "items": rows })
}
