//! Clusters: group rows by (candidate key, status), describe each group, pick exemplars, order the list.

use super::tiers::Best;
use super::{money, ClusterOut, Ctx, Exemplar, Request, Status, Tier};
use crate::cache::Txn;
use std::collections::{BTreeMap, HashMap};

fn status_of(t: &Txn, target: i64) -> Status {
    match t.category_id {
        None => Status::Uncategorized,
        Some(c) if c == target => Status::Same,
        Some(_) => Status::Conflict,
    }
}

fn status_key(s: Status) -> &'static str {
    match s {
        Status::Uncategorized => "u",
        Status::Conflict => "c",
        Status::Same => "s",
    }
}

/// One group before it is described.
struct Raw {
    tier: Tier,
    conf: f64,
    reason: String,
    status: Status,
    members: Vec<usize>,
}

/// Up to four distinct rows that show a cluster: earliest, latest, median amount, farthest from the median.
fn exemplars(ctx: &Ctx, members: &[usize]) -> Vec<Exemplar> {
    let txn = |i: usize| ctx.txn(i);
    let mut by_date = members.to_vec();
    by_date.sort_by(|&a, &b| (&txn(a).date, txn(a).id).cmp(&(&txn(b).date, txn(b).id)));
    let earliest = by_date[0];
    let latest = by_date[by_date.len() - 1];

    let mut by_amt = members.to_vec();
    by_amt.sort_by(|&a, &b| {
        txn(a).amount.partial_cmp(&txn(b).amount).unwrap_or(std::cmp::Ordering::Equal).then(txn(a).id.cmp(&txn(b).id))
    });
    let mid = by_amt[(by_amt.len() - 1) / 2];
    let med_amt = txn(mid).amount;
    let outlier = members.iter().copied().fold(None::<usize>, |best, i| match best {
        None => Some(i),
        Some(b) => {
            let (di, db) = ((txn(i).amount - med_amt).abs(), (txn(b).amount - med_amt).abs());
            if di > db || (di == db && txn(i).id < txn(b).id) {
                Some(i)
            } else {
                Some(b)
            }
        }
    });

    let mut picked: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let order = [(Some(earliest), "earliest"), (Some(latest), "latest"), (Some(mid), "median"), (outlier, "outlier")];
    for (i, why) in order {
        let Some(i) = i else { continue };
        if picked.contains(&i) {
            continue;
        }
        picked.push(i);
        let t = txn(i);
        out.push(Exemplar {
            id: t.id,
            date: t.date.clone(),
            payee: t.payee.clone(),
            amount: t.amount,
            account: t.account.clone(),
            category_path: ctx.cache.category_path(t).map(str::to_string),
            why,
        });
    }
    out
}

pub(crate) fn build(ctx: &Ctx, cands: &[Option<Best>], req: &Request) -> Vec<ClusterOut> {
    let mut raw: BTreeMap<String, Raw> = BTreeMap::new();
    for i in 0..ctx.cache.txns.len() {
        let t = ctx.txn(i);
        let (key, tier, conf, reason) = if ctx.seed[i] {
            let k = format!("seed|{}|{}", ctx.norm_id[i], super::is_income(t) as u8);
            (k, Tier::Seed, 1.0, "selected rows".to_string())
        } else if let Some(b) = &cands[i] {
            (b.key.clone(), b.tier, b.conf, b.reason.clone())
        } else {
            continue;
        };
        let st = status_of(t, req.category_id);
        let full = format!("{}|{}", key, status_key(st));
        let e = raw.entry(full).or_insert(Raw { tier, conf: f64::MAX, reason, status: st, members: Vec::new() });
        if conf < e.conf {
            e.conf = conf;
        }
        e.members.push(i);
    }

    let mut out: Vec<ClusterOut> = Vec::with_capacity(raw.len());
    for (_, r) in raw {
        let mut norm_count: HashMap<u32, usize> = HashMap::new();
        for &i in &r.members {
            *norm_count.entry(ctx.norm_id[i]).or_insert(0) += 1;
        }
        let label_id = norm_count
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| ctx.norms[*b.0 as usize].cmp(&ctx.norms[*a.0 as usize])))
            .map(|(id, _)| *id)
            .unwrap_or(0);
        let label = ctx.norms[label_id as usize].clone();

        let txn = |i: usize| ctx.txn(i);
        let count = r.members.len();
        let total: f64 = r.members.iter().map(|&i| txn(i).amount).sum();
        let date_min = r.members.iter().map(|&i| txn(i).date.as_str()).min().unwrap_or("").to_string();
        let date_max = r.members.iter().map(|&i| txn(i).date.as_str()).max().unwrap_or("").to_string();
        let amount_min = r.members.iter().map(|&i| txn(i).amount).fold(f64::INFINITY, f64::min);
        let amount_max = r.members.iter().map(|&i| txn(i).amount).fold(f64::NEG_INFINITY, f64::max);

        let mut by_date_desc = r.members.clone();
        by_date_desc.sort_by(|&a, &b| (&txn(b).date, txn(b).id).cmp(&(&txn(a).date, txn(a).id)));
        let txn_ids: Vec<i64> = by_date_desc.iter().map(|&i| txn(i).id).collect();

        let mut conflict_categories: Vec<String> = Vec::new();
        if r.status == Status::Conflict {
            for &i in &r.members {
                if let Some(p) = ctx.cache.category_path(txn(i)) {
                    if !conflict_categories.iter().any(|c| c == p) {
                        conflict_categories.push(p.to_string());
                    }
                }
            }
            conflict_categories.sort();
        }

        // Seed and exact clusters are the rows the user named or the same payee: selected unless already set
        // differently (then only with overwrite). Fuzzy, recurring and amount-band matches are suggestions.
        let trusted = matches!(r.tier, Tier::Seed | Tier::Exact);
        let default_selected = trusted
            && match r.status {
                Status::Uncategorized => true,
                Status::Conflict => req.overwrite,
                Status::Same => false,
            };

        out.push(ClusterOut {
            id: String::new(),
            label,
            tier: r.tier,
            confidence: (r.conf * 1e6).round() / 1e6,
            reason: r.reason,
            count,
            total: money(total),
            date_min,
            date_max,
            amount_min,
            amount_max,
            status: r.status,
            conflict_categories,
            default_selected,
            exemplars: {
                let mut asc = r.members.clone();
                asc.sort_by(|&a, &b| (&txn(a).date, txn(a).id).cmp(&(&txn(b).date, txn(b).id)));
                exemplars(ctx, &asc)
            },
            txn_ids,
        });
    }

    out.sort_by(|a, b| {
        (a.tier != Tier::Seed)
            .cmp(&(b.tier != Tier::Seed))
            .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.txn_ids.iter().min().cmp(&b.txn_ids.iter().min()))
    });
    for (k, c) in out.iter_mut().enumerate() {
        c.id = format!("c{}", k + 1);
    }
    out
}
