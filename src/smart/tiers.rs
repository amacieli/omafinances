//! Candidate search. For every non-seed row, the best candidate (if any) across the four tiers. Index-based: the
//! work is one pass over the rows plus postings lookups; no row is compared with every other row.

use super::similarity::{tokens, TrigramIndex};
use super::{is_income, Ctx, Groups, Tier};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub(crate) struct Best {
    pub tier: Tier,
    pub conf: f64,
    /// Cluster key before the status split. Rows with equal keys (and status) share a cluster.
    pub key: String,
    pub reason: String,
}

/// Keep the better candidate: higher confidence, then the tier rank.
fn offer(slot: &mut Option<Best>, cand: Best) {
    let better = match slot {
        None => true,
        Some(b) => {
            cand.conf > b.conf + 1e-12 || ((cand.conf - b.conf).abs() <= 1e-12 && cand.tier.rank() < b.tier.rank())
        }
    };
    if better {
        *slot = Some(cand);
    }
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Best candidate per row (`None` for seeds and for rows no tier matched).
pub(crate) fn candidates(ctx: &Ctx, groups: &Groups, threshold: f64) -> Vec<Option<Best>> {
    let n = ctx.cache.txns.len();
    let mut out: Vec<Option<Best>> = vec![None; n];

    // (a) exact: the same normalized payee and sign as a seed group.
    for i in 0..n {
        if ctx.seed[i] {
            continue;
        }
        let t = ctx.txn(i);
        let k = (ctx.norm_id[i], is_income(t));
        if groups.0.contains_key(&k) {
            offer(
                &mut out[i],
                Best {
                    tier: Tier::Exact,
                    conf: 1.0,
                    key: format!("exact|{}|{}", k.0, k.1 as u8),
                    reason: "same normalized payee".into(),
                },
            );
        }
    }

    // (b) fuzzy: payees whose trigram-Jaccard/Jaro-Winkler score against a seed payee reaches the threshold.
    // Best (score, seed payee) per (candidate payee, sign).
    let index = TrigramIndex::build(&ctx.norms);
    let mut seed_norms: Vec<u32> = groups.0.keys().map(|k| k.0).collect();
    seed_norms.sort_unstable();
    seed_norms.dedup();
    let mut best_f: HashMap<(u32, bool), (f64, u32)> = HashMap::new();
    for g in seed_norms {
        let signs: Vec<bool> = [false, true].into_iter().filter(|s| groups.0.contains_key(&(g, *s))).collect();
        for (c, score) in index.similar(&ctx.norms, g, threshold) {
            for &s in &signs {
                let e = best_f.entry((c, s)).or_insert((score, g));
                if score > e.0 {
                    *e = (score, g);
                }
            }
        }
    }
    for i in 0..n {
        if ctx.seed[i] {
            continue;
        }
        let nid = ctx.norm_id[i];
        let s = is_income(ctx.txn(i));
        if let Some(&(score, g)) = best_f.get(&(nid, s)) {
            offer(
                &mut out[i],
                Best {
                    tier: Tier::Fuzzy,
                    conf: score,
                    key: format!("fuzzy|{}|{}", nid, s as u8),
                    reason: format!("payee {:.2} similar to '{}'", score, ctx.norms[g as usize]),
                },
            );
        }
    }

    // (c) recurring: the same Lunch Money recurring series as a seed row, same sign.
    let mut rec: HashSet<(i64, bool)> = HashSet::new();
    for &i in &ctx.seeds {
        if let Some(r) = ctx.txn(i).recurring_id {
            rec.insert((r, is_income(ctx.txn(i))));
        }
    }
    const RECURRING_CONF: f64 = 0.9;
    if RECURRING_CONF >= threshold && !rec.is_empty() {
        for i in 0..n {
            if ctx.seed[i] {
                continue;
            }
            let t = ctx.txn(i);
            let s = is_income(t);
            if let Some(r) = t.recurring_id {
                if rec.contains(&(r, s)) {
                    offer(
                        &mut out[i],
                        Best {
                            tier: Tier::Recurring,
                            conf: RECURRING_CONF,
                            key: format!("recurring|{}|{}", r, s as u8),
                            reason: "same recurring id".into(),
                        },
                    );
                }
            }
        }
    }

    // (d) amount band: shares a non-stop payee word with a seed group and is within 25% of the seed median.
    // Confidence 0.6 (at the edge of the band) to 0.8 (equal to the median).
    struct Grp {
        sign: bool,
        median: f64,
        name: String,
    }
    let mut grps: Vec<Grp> = Vec::new();
    let mut by_tok: HashMap<String, Vec<usize>> = HashMap::new();
    for ((nid, s), members) in &groups.0 {
        let mut amts: Vec<f64> = members.iter().map(|&i| ctx.txn(i).amount.abs()).collect();
        amts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let gi = grps.len();
        for tok in tokens(&ctx.norms[*nid as usize]) {
            by_tok.entry(tok).or_default().push(gi);
        }
        grps.push(Grp { sign: *s, median: median(&amts), name: ctx.norms[*nid as usize].clone() });
    }
    if !grps.is_empty() {
        let mut row_toks: HashMap<u32, Vec<String>> = HashMap::new();
        for i in 0..n {
            if ctx.seed[i] {
                continue;
            }
            let nid = ctx.norm_id[i];
            let t = ctx.txn(i);
            let s = is_income(t);
            let toks = row_toks.entry(nid).or_insert_with(|| tokens(&ctx.norms[nid as usize]));
            let a = t.amount.abs();
            let mut seen: Vec<usize> = Vec::new();
            for tok in toks.iter() {
                let Some(gs) = by_tok.get(tok) else { continue };
                for &gi in gs {
                    if grps[gi].sign != s || seen.contains(&gi) {
                        continue;
                    }
                    seen.push(gi);
                    let med = grps[gi].median;
                    if med <= 0.0 {
                        continue;
                    }
                    let dev = (a - med).abs() / med;
                    if dev <= 0.25 {
                        let conf = 0.6 + 0.2 * (1.0 - dev / 0.25);
                        if conf >= threshold {
                            offer(
                                &mut out[i],
                                Best {
                                    tier: Tier::AmountBand,
                                    conf,
                                    key: format!("amount|{}|{}|{}", nid, s as u8, gi),
                                    reason: format!("amount near '{}' and shares a payee word", grps[gi].name),
                                },
                            );
                        }
                    }
                }
            }
        }
    }
    out
}
