//! Dashboard: monthly out/in, spending by category and payee, totals, and what the transfer pairing removed.

use super::{day, money, Model};
use crate::cache::Cache;
use chrono::{Datelike, Months, NaiveDate};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub const MAX_MONTHS: u32 = 60;
const TOP_CATEGORIES: usize = 15;
const TOP_PAYEES: usize = 10;

pub struct Params {
    pub months: u32,
    pub exclude_transfers: bool,
}

pub fn report(cache: &Cache, model: &Model, today: NaiveDate, p: &Params) -> Value {
    let this_month = today.with_day(1).unwrap_or(today);
    let from = this_month.checked_sub_months(Months::new(p.months - 1)).unwrap_or(this_month);
    let to = this_month.checked_add_months(Months::new(1)).unwrap_or(this_month) - chrono::Days::new(1);
    let ym = |d: NaiveDate| d.year() * 12 + d.month0() as i32;
    let start = ym(from);

    let mut months: Vec<(f64, f64)> = vec![(0.0, 0.0); p.months as usize];
    let mut cats: BTreeMap<Option<i64>, (f64, usize)> = BTreeMap::new();
    let mut payees: HashMap<&str, (f64, usize)> = HashMap::new();
    let (mut tot_out, mut tot_in) = (0.0, 0.0);
    let mut in_window = vec![false; cache.txns.len()];
    for (i, t) in cache.txns.iter().enumerate() {
        if day(t).is_none() {
            continue;
        }
        let slot = (t.date_key / 10000) as i32 * 12 + (t.date_key / 100 % 100) as i32 - 1 - start;
        if slot < 0 || slot >= p.months as i32 {
            continue;
        }
        in_window[i] = true;
        if p.exclude_transfers && model.transfers.is_transfer[i] {
            continue;
        }
        if t.amount > 0.0 {
            months[slot as usize].0 += t.amount;
            tot_out += t.amount;
            let c = cats.entry(t.category_id).or_insert((0.0, 0));
            c.0 += t.amount;
            c.1 += 1;
            let name = if model.norms[i].is_empty() { "(no payee)" } else { model.norms[i].as_str() };
            let e = payees.entry(name).or_insert((0.0, 0));
            e.0 += t.amount;
            e.1 += 1;
        } else if t.amount < 0.0 {
            months[slot as usize].1 += -t.amount;
            tot_in += -t.amount;
        }
    }

    let month_rows: Vec<Value> = months
        .iter()
        .enumerate()
        .map(|(k, &(o, n))| {
            let d = from.checked_add_months(Months::new(k as u32)).unwrap_or(from);
            json!({ "month": d.format("%Y-%m").to_string(), "out": money(o), "in": money(n), "net": money(n - o) })
        })
        .collect();

    let uncategorized = cats.remove(&None);
    let mut ranked: Vec<(String, i64, f64, usize)> = cats
        .into_iter()
        .filter_map(|(id, (o, c))| id.map(|id| (cache.node_pos(id).map(|p| cache.categories[p as usize].path.clone()).unwrap_or_default(), id, o, c)))
        .collect();
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    let mut by_category: Vec<Value> = ranked
        .iter()
        .take(TOP_CATEGORIES)
        .map(|(path, id, o, c)| json!({ "category_id": id, "path": path, "out": money(*o), "count": c }))
        .collect();
    if ranked.len() > TOP_CATEGORIES {
        let o: f64 = ranked[TOP_CATEGORIES..].iter().map(|r| r.2).sum();
        let c: usize = ranked[TOP_CATEGORIES..].iter().map(|r| r.3).sum();
        by_category.push(json!({ "category_id": null, "path": "Other", "out": money(o), "count": c }));
    }
    if let Some((o, c)) = uncategorized {
        by_category.push(json!({ "category_id": null, "path": "Uncategorized", "out": money(o), "count": c }));
    }

    let mut pv: Vec<(&str, f64, usize)> = payees.into_iter().map(|(n, (o, c))| (n, o, c)).collect();
    pv.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(b.0)));
    let top_payees: Vec<Value> =
        pv.iter().take(TOP_PAYEES).map(|(n, o, c)| json!({ "payee": n, "out": money(*o), "count": c })).collect();

    let (mut pairs, mut pair_total) = (0usize, 0.0);
    if p.exclude_transfers {
        for pr in &model.transfers.pairs {
            if in_window[pr.out] || in_window[pr.inn] {
                pairs += 1;
                pair_total += pr.amount;
            }
        }
    }
    json!({
        "from": from.format("%Y-%m-%d").to_string(),
        "to": to.format("%Y-%m-%d").to_string(),
        "months": month_rows,
        "by_category": by_category,
        "top_payees": top_payees,
        "totals": { "out": money(tot_out), "in": money(tot_in), "net": money(tot_in - tot_out) },
        "transfers_excluded": { "pairs": pairs, "total": money(pair_total) },
    })
}
