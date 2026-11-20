//! Categorize with preview. Given the selected rows and a target category, find the other rows that look like them
//! (same normalized payee, fuzzy payee, same recurring series, similar amount with a shared payee word), group
//! them into clusters, and describe each cluster for the UI. Pure: no database and no journal here. The apply
//! path lives in `db::rules`.
//!
//! Tiers, in priority order for a row that several tiers match: the highest confidence wins, ties go to
//! exact, recurring, fuzzy, amount_band. Seed rows are always in a `seed` cluster and are never candidates.
//! Income and expense never match each other (the sign of the amount is part of every key).

pub mod cluster;
pub mod normalize;
pub mod similarity;
pub mod tiers;

#[cfg(test)]
mod tests;

use crate::cache::{Cache, Txn};
use normalize::{normalize, Aliases};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub const MAX_IDS: usize = 500;
pub const DEFAULT_THRESHOLD: f64 = 0.75;

/// Request validation and lookup failures. The API maps these to 400 and 404.
#[derive(Debug, Clone, PartialEq)]
pub enum SmartError {
    Invalid(String),
    NotFound(&'static str),
}

/// A validated preview request. `ids` may repeat and may name rows that do not exist.
#[derive(Debug, Clone)]
pub struct Request {
    pub ids: Vec<i64>,
    pub category_id: i64,
    pub threshold: f64,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Seed,
    Exact,
    Fuzzy,
    Recurring,
    AmountBand,
}

impl Tier {
    /// Tie-break when two tiers offer the same confidence. Lower wins.
    pub fn rank(self) -> u8 {
        match self {
            Tier::Seed | Tier::Exact => 0,
            Tier::Recurring => 1,
            Tier::Fuzzy => 2,
            Tier::AmountBand => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Uncategorized,
    Conflict,
    Same,
}

#[derive(Debug, Clone, Serialize)]
pub struct Exemplar {
    pub id: i64,
    pub date: String,
    pub payee: String,
    pub amount: f64,
    pub account: Option<String>,
    pub category_path: Option<String>,
    pub why: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterOut {
    pub id: String,
    pub label: String,
    pub tier: Tier,
    pub confidence: f64,
    pub reason: String,
    pub count: usize,
    pub total: f64,
    pub date_min: String,
    pub date_max: String,
    pub amount_min: f64,
    pub amount_max: f64,
    pub status: Status,
    pub conflict_categories: Vec<String>,
    pub default_selected: bool,
    pub exemplars: Vec<Exemplar>,
    pub txn_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryRef {
    pub id: i64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub matched: usize,
    pub selected: usize,
    pub selected_total: f64,
    pub uncategorized_total: usize,
    pub conflicts: usize,
    pub same: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub category: CategoryRef,
    pub threshold: f64,
    pub clusters: Vec<ClusterOut>,
    pub summary: Summary,
}

/// Income (money in) has a negative amount. Every match is restricted to one sign.
pub(crate) fn is_income(t: &Txn) -> bool {
    t.amount < 0.0
}

pub fn money(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Seed groups: (normalized payee id, income?) -> indices into `Cache::txns`.
pub(crate) struct Groups(pub BTreeMap<(u32, bool), Vec<usize>>);

/// Per-request view of the cache: normalized payee per transaction, and which rows are seeds.
pub(crate) struct Ctx<'a> {
    pub cache: &'a Cache,
    /// Distinct normalized payees, by id.
    pub norms: Vec<String>,
    /// Normalized payee id per transaction (same indexing as `cache.txns`).
    pub norm_id: Vec<u32>,
    pub seed: Vec<bool>,
    /// Seed indices, ascending.
    pub seeds: Vec<usize>,
}

impl<'a> Ctx<'a> {
    fn build(cache: &'a Cache, aliases: &Aliases, seeds: Vec<usize>) -> Ctx<'a> {
        // One id per distinct *normalized* payee, so raw variants that normalize alike share a group. The raw
        // payee is normalized once (`raw_ids`); the normalized text decides the id (`norm_ids`).
        let mut raw_ids: HashMap<&str, u32> = HashMap::new();
        let mut norm_ids: HashMap<String, u32> = HashMap::new();
        let mut norms: Vec<String> = Vec::new();
        let mut norm_id = Vec::with_capacity(cache.txns.len());
        for t in &cache.txns {
            let key = t.payee_lc.as_str();
            let id = match raw_ids.get(key) {
                Some(&id) => id,
                None => {
                    let n = normalize(key, aliases);
                    let id = match norm_ids.get(&n) {
                        Some(&id) => id,
                        None => {
                            let id = norms.len() as u32;
                            norms.push(n.clone());
                            norm_ids.insert(n, id);
                            id
                        }
                    };
                    raw_ids.insert(key, id);
                    id
                }
            };
            norm_id.push(id);
        }
        let mut seed = vec![false; cache.txns.len()];
        for &i in &seeds {
            seed[i] = true;
        }
        Ctx { cache, norms, norm_id, seed, seeds }
    }

    pub fn txn(&self, i: usize) -> &'a Txn {
        &self.cache.txns[i]
    }
}

/// Run the preview. Read-only.
pub fn preview(cache: &Cache, aliases: &Aliases, req: &Request) -> Result<Preview, SmartError> {
    if req.ids.is_empty() || req.ids.len() > MAX_IDS {
        return Err(SmartError::Invalid(format!("ids must hold 1 to {MAX_IDS} transaction ids")));
    }
    if !(0.5..=1.0).contains(&req.threshold) {
        return Err(SmartError::Invalid("threshold must be between 0.5 and 1.0".into()));
    }
    let pos = cache.node_pos(req.category_id).ok_or(SmartError::NotFound("category not found"))?;
    let path = cache.categories[pos as usize].path.clone();

    let mut ids = req.ids.clone();
    ids.sort_unstable();
    ids.dedup();
    let mut seeds: Vec<usize> = ids
        .iter()
        .filter_map(|id| cache.txns.binary_search_by_key(id, |t| t.id).ok())
        .collect();
    seeds.sort_unstable();
    seeds.dedup();
    if seeds.is_empty() {
        return Err(SmartError::Invalid("none of the ids exist".into()));
    }

    let ctx = Ctx::build(cache, aliases, seeds);
    let mut groups: BTreeMap<(u32, bool), Vec<usize>> = BTreeMap::new();
    for &i in &ctx.seeds {
        groups.entry((ctx.norm_id[i], is_income(ctx.txn(i)))).or_default().push(i);
    }
    let groups = Groups(groups);

    let cands = tiers::candidates(&ctx, &groups, req.threshold);
    let clusters = cluster::build(&ctx, &cands, req);

    let mut summary = Summary {
        matched: 0,
        selected: 0,
        selected_total: 0.0,
        uncategorized_total: cache.uncategorized,
        conflicts: 0,
        same: 0,
    };
    let mut sel_total = 0.0;
    for c in &clusters {
        summary.matched += c.count;
        if c.default_selected {
            summary.selected += c.count;
            sel_total += c.total;
        }
        match c.status {
            Status::Conflict => summary.conflicts += c.count,
            Status::Same => summary.same += c.count,
            Status::Uncategorized => {}
        }
    }
    summary.selected_total = money(sel_total);

    Ok(Preview {
        category: CategoryRef { id: req.category_id, path },
        threshold: req.threshold,
        clusters,
        summary,
    })
}
