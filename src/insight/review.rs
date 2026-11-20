//! The review queue: uncategorized rows, best suggestions first, or (bootstrap) biggest payee groups first.

use super::suggest::{options, Opt};
use super::Model;
use crate::cache::Cache;
use std::collections::HashMap;

pub const MAX_LIMIT: usize = 200;

pub struct Row {
    pub idx: usize,
    pub payee_count: usize,
    pub options: Vec<Opt>,
}

pub struct Queue {
    pub suggest_mode: bool,
    pub remaining: usize,
    pub rows: Vec<Row>,
}

pub fn queue(cache: &Cache, model: &Model, limit: usize, offset: usize) -> Queue {
    let uncat: Vec<usize> = (0..cache.txns.len()).filter(|&i| cache.txns[i].category_id.is_none()).collect();
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for &i in &uncat {
        *freq.entry(model.norms[i].as_str()).or_insert(0) += 1;
    }
    let suggest_mode = model.suggest(cache).enabled;
    let newest = |i: usize| std::cmp::Reverse((cache.txns[i].date_key, cache.txns[i].id));
    let mut rows: Vec<Row>;
    if suggest_mode {
        rows = uncat
            .iter()
            .map(|&i| Row { idx: i, payee_count: freq[model.norms[i].as_str()], options: options(model, cache, i, true) })
            .collect();
        let top = |r: &Row| r.options.first().map_or(0.0, |o| o.confidence);
        rows.sort_by(|a, b| {
            top(b).partial_cmp(&top(a)).unwrap_or(std::cmp::Ordering::Equal).then(newest(a.idx).cmp(&newest(b.idx)))
        });
    } else {
        let mut order = uncat.clone();
        order.sort_by(|&a, &b| {
            let (na, nb) = (model.norms[a].as_str(), model.norms[b].as_str());
            freq[nb].cmp(&freq[na]).then(na.cmp(nb)).then(newest(a).cmp(&newest(b)))
        });
        let page: Vec<usize> = order.into_iter().skip(offset).take(limit).collect();
        rows = page
            .into_iter()
            .map(|i| Row { idx: i, payee_count: freq[model.norms[i].as_str()], options: options(model, cache, i, false) })
            .collect();
        return Queue { suggest_mode, remaining: uncat.len(), rows };
    }
    let rows = rows.into_iter().skip(offset).take(limit).collect();
    Queue { suggest_mode, remaining: uncat.len(), rows }
}

pub fn report(cache: &Cache, q: &Queue) -> serde_json::Value {
    let rows: Vec<_> = q
        .rows
        .iter()
        .map(|r| {
            let t = &cache.txns[r.idx];
            serde_json::json!({
                "id": t.id,
                "date": t.date,
                "payee": t.payee,
                "amount": super::money(t.amount),
                "account": t.account,
                "payee_count": r.payee_count,
                "options": r.options,
            })
        })
        .collect();
    serde_json::json!({ "mode": if q.suggest_mode { "suggest" } else { "frequency" }, "remaining": q.remaining, "rows": rows })
}
