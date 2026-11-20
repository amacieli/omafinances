//! Category suggestions: enabled rules first, then multinomial naive Bayes over the labeled rows, gated by a
//! deterministic 5-fold cross-validation (fold = id % 5) that is always measured.
//!
//! Scoring uses sparse postings: for class c, `score = ln(docs_c+1) + sum_f ln(n_fc+1) - k * ln(N_c + V)` where only
//! features seen in training (k of them) count, `N_c` is the class's feature total and `V` the vocabulary size
//! (Laplace alpha = 1). Fold counts are kept per posting, so held-out scoring subtracts the fold instead of retraining.

use super::Model;
use crate::cache::Cache;
use crate::db::rules::RuleRow;
use crate::smart::similarity::tokens;
use serde::Serialize;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

pub const MIN_LABELED: usize = 30;
pub const MIN_TOP3: f64 = 0.70;
pub const MIN_CONFIDENCE: f64 = 0.30;
pub const MAX_OPTIONS: usize = 3;
const FOLDS: usize = 5;
/// "Fold" index meaning no row is held out.
const FULL: usize = FOLDS;

#[derive(Default)]
struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}
type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Accuracy {
    pub labeled: usize,
    pub folds: usize,
    pub top1: f64,
    pub top3: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Opt {
    pub category_id: i64,
    pub path: String,
    pub confidence: f64,
    pub source: &'static str,
}

struct Post {
    class: u32,
    tot: u32,
    cnt: [u32; FOLDS],
}

pub struct Suggest {
    classes: Vec<i64>,
    feats: Vec<u32>,
    off: Vec<u32>,
    post: Vec<Vec<Post>>,
    /// `[fold][class]`, fold `FULL` = everything.
    prior: Vec<Vec<f64>>,
    ln_d: Vec<Vec<f64>>,
    ln_tab: Vec<f64>,
    pub accuracy: Accuracy,
    pub enabled: bool,
    pub reason: Option<String>,
}

fn kind(k: u32, v: u32) -> u32 {
    (k << 28) | (v & 0x0FFF_FFFF)
}

fn amount_bucket(a: f64) -> u32 {
    let m = a.abs();
    let b = if m < 0.01 { 0 } else { ((m.log10() * 2.0).floor() as i32 + 4).clamp(0, 40) } as u32;
    if a < 0.0 {
        b + 64
    } else {
        b
    }
}

fn round_down(x: f64, places: i32) -> f64 {
    let p = 10f64.powi(places);
    (x * p).floor() / p
}

impl Suggest {
    pub fn train(cache: &Cache, norms: &[String]) -> Suggest {
        let n = cache.txns.len();
        let mut dense: FxMap<u32, u32> = FxMap::default();
        let mut whole_ids: HashMap<&str, u32> = HashMap::new();
        let mut tok_ids: HashMap<String, u32> = HashMap::new();
        let mut lm_ids: HashMap<&str, u32> = HashMap::new();
        let mut payee_feats: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut feats: Vec<u32> = Vec::with_capacity(n * 8);
        let mut off: Vec<u32> = Vec::with_capacity(n + 1);
        off.push(0);
        for (i, t) in cache.txns.iter().enumerate() {
            let norm = norms[i].as_str();
            if !payee_feats.contains_key(norm) {
                let mut v = Vec::new();
                if !norm.is_empty() {
                    let next = whole_ids.len() as u32;
                    v.push(kind(1, *whole_ids.entry(norm).or_insert(next)));
                    for tok in tokens(norm) {
                        let next = tok_ids.len() as u32;
                        v.push(kind(2, *tok_ids.entry(tok).or_insert(next)));
                    }
                }
                payee_feats.insert(norm, v);
            }
            let mut raw: Vec<u32> = payee_feats[norm].clone();
            raw.push(kind(3, amount_bucket(t.amount)));
            raw.push(kind(4, t.account_id.unwrap_or(0) as u32));
            let dom = if t.date_key == 0 { 0 } else { ((t.date_key % 100).saturating_sub(1) / 7).min(3) + 1 };
            raw.push(kind(5, dom));
            if let Some(lm) = t.lm_category.as_deref().filter(|s| !s.is_empty()) {
                let next = lm_ids.len() as u32;
                raw.push(kind(6, *lm_ids.entry(lm).or_insert(next)));
            }
            for r in raw {
                let next = dense.len() as u32;
                feats.push(*dense.entry(r).or_insert(next));
            }
            off.push(feats.len() as u32);
        }

        let mut classes: Vec<i64> = cache.txns.iter().filter_map(|t| t.category_id).collect();
        classes.sort_unstable();
        classes.dedup();
        let class_of: FxMap<i64, u32> = classes.iter().enumerate().map(|(i, &c)| (c, i as u32)).collect();
        let nc = classes.len();

        let mut post: Vec<Vec<Post>> = (0..dense.len()).map(|_| Vec::new()).collect();
        // Held-out totals per class and fold: feature occurrences and documents.
        let mut feat_held = vec![[0u64; FOLDS]; nc];
        let mut doc_held = vec![[0u64; FOLDS]; nc];
        let mut labeled = 0usize;
        for (i, t) in cache.txns.iter().enumerate() {
            let Some(cat) = t.category_id else { continue };
            let class = class_of[&cat];
            let fold = t.id.rem_euclid(FOLDS as i64) as usize;
            labeled += 1;
            let row = &feats[off[i] as usize..off[i + 1] as usize];
            feat_held[class as usize][fold] += row.len() as u64;
            doc_held[class as usize][fold] += 1;
            for &f in row {
                let ps = &mut post[f as usize];
                match ps.iter().position(|p| p.class == class) {
                    Some(k) => {
                        ps[k].tot += 1;
                        ps[k].cnt[fold] += 1;
                    }
                    None => {
                        let mut cnt = [0u32; FOLDS];
                        cnt[fold] = 1;
                        ps.push(Post { class, tot: 1, cnt });
                    }
                }
            }
        }

        // Per-fold vocabulary and class totals.
        let mut vocab = [0usize; FOLDS + 1];
        let mut class_feat = vec![[0u64; FOLDS + 1]; nc];
        let mut class_doc = vec![[0u64; FOLDS + 1]; nc];
        for ps in &post {
            for fold in 0..=FULL {
                let mut known = false;
                for p in ps {
                    let held = if fold < FOLDS { p.cnt[fold] } else { 0 };
                    if p.tot > held {
                        known = true;
                    }
                }
                if known {
                    vocab[fold] += 1;
                }
            }
        }
        for c in 0..nc {
            let tf: u64 = feat_held[c].iter().sum();
            let td: u64 = doc_held[c].iter().sum();
            for fold in 0..=FULL {
                let (hf, hd) = if fold < FOLDS { (feat_held[c][fold], doc_held[c][fold]) } else { (0, 0) };
                class_feat[c][fold] = tf - hf;
                class_doc[c][fold] = td - hd;
            }
        }
        let mut prior = vec![vec![0.0; nc]; FOLDS + 1];
        let mut ln_d = vec![vec![0.0; nc]; FOLDS + 1];
        for fold in 0..=FULL {
            for c in 0..nc {
                prior[fold][c] = ((class_doc[c][fold] + 1) as f64).ln();
                ln_d[fold][c] = ((class_feat[c][fold] + vocab[fold] as u64).max(1) as f64).ln();
            }
        }
        let ln_tab: Vec<f64> = (0..8192u32).map(|n| ((n + 1) as f64).ln()).collect();

        let mut s = Suggest {
            classes,
            feats,
            off,
            post,
            prior,
            ln_d,
            ln_tab,
            accuracy: Accuracy { labeled, folds: FOLDS, top1: 0.0, top3: 0.0 },
            enabled: false,
            reason: None,
        };
        s.cross_validate(cache);
        s
    }

    fn cross_validate(&mut self, cache: &Cache) {
        let mut hit1 = 0usize;
        let mut hit3 = 0usize;
        let mut scores = vec![0.0; self.classes.len()];
        let labeled = self.accuracy.labeled;
        if !self.classes.is_empty() {
            for (i, t) in cache.txns.iter().enumerate() {
                let Some(cat) = t.category_id else { continue };
                let want = self.classes.binary_search(&cat).unwrap_or(usize::MAX);
                let fold = t.id.rem_euclid(FOLDS as i64) as usize;
                self.score(i, fold, &mut scores);
                let top = top3(&scores);
                if top[0] == Some(want) {
                    hit1 += 1;
                }
                if top.contains(&Some(want)) {
                    hit3 += 1;
                }
            }
        }
        let rate = |h: usize| if labeled == 0 { 0.0 } else { (h as f64 / labeled as f64 * 10_000.0).round() / 10_000.0 };
        self.accuracy.top1 = rate(hit1);
        self.accuracy.top3 = rate(hit3);
        if labeled < MIN_LABELED {
            self.reason = Some(format!("only {labeled} labeled rows (need {MIN_LABELED})"));
        } else if self.accuracy.top3 < MIN_TOP3 {
            self.reason = Some(format!("top-3 accuracy {:.2} below {:.2}", round_down(self.accuracy.top3, 2), MIN_TOP3));
        } else {
            self.enabled = true;
        }
    }

    fn ln_n1(&self, n: u32) -> f64 {
        self.ln_tab.get(n as usize).copied().unwrap_or_else(|| ((n + 1) as f64).ln())
    }

    /// Log scores of every class for transaction `i` with `fold` held out.
    fn score(&self, i: usize, fold: usize, scores: &mut [f64]) {
        for s in scores.iter_mut() {
            *s = 0.0;
        }
        let mut k = 0u32;
        for &f in &self.feats[self.off[i] as usize..self.off[i + 1] as usize] {
            let mut known = false;
            for p in &self.post[f as usize] {
                let held = if fold < FOLDS { p.cnt[fold] } else { 0 };
                let n = p.tot - held;
                if n > 0 {
                    scores[p.class as usize] += self.ln_n1(n);
                    known = true;
                }
            }
            if known {
                k += 1;
            }
        }
        for (c, s) in scores.iter_mut().enumerate() {
            *s += self.prior[fold][c] - k as f64 * self.ln_d[fold][c];
        }
    }

    /// Top three `(category id, posterior)` for transaction `i`, best first.
    pub fn predict(&self, i: usize) -> Vec<(i64, f64)> {
        if self.classes.is_empty() {
            return Vec::new();
        }
        let mut scores = vec![0.0; self.classes.len()];
        self.score(i, FULL, &mut scores);
        let max = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = scores.iter().map(|s| (s - max).exp()).sum();
        top3(&scores).iter().flatten().map(|&c| (self.classes[c], (scores[c] - max).exp() / z)).collect()
    }
}

/// Indices of the three best scores; ties keep the lower index (lower category id).
fn top3(scores: &[f64]) -> [Option<usize>; 3] {
    let mut best: [Option<usize>; 3] = [None; 3];
    for (i, &s) in scores.iter().enumerate() {
        let mut slot = 3;
        for (j, b) in best.iter().enumerate() {
            if b.map_or(true, |bi| s > scores[bi]) {
                slot = j;
                break;
            }
        }
        if slot < 3 {
            for j in (slot + 1..3).rev() {
                best[j] = best[j - 1];
            }
            best[slot] = Some(i);
        }
    }
    best
}

/// An enabled rule that can fire: it has a category in the tree and a payee condition.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub id: i64,
    pub payees: Vec<String>,
    pub amount_min: Option<f64>,
    pub amount_max: Option<f64>,
    pub category_id: i64,
}

pub fn compile_rules(cache: &Cache, rules: &[RuleRow]) -> Vec<CompiledRule> {
    let mut out = Vec::new();
    for r in rules {
        let Some(cat) = r.category_id.filter(|&c| cache.node_pos(c).is_some()) else { continue };
        if !r.enabled {
            continue;
        }
        let mut payees: Vec<String> = Vec::new();
        if let Some(p) = r.matched.get("payee_norm").and_then(|v| v.as_str()) {
            payees.push(p.to_string());
        }
        if let Some(a) = r.matched.get("payee_norm_any").and_then(|v| v.as_array()) {
            payees.extend(a.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
        if payees.is_empty() {
            continue;
        }
        out.push(CompiledRule {
            id: r.id,
            payees,
            amount_min: r.matched.get("amount_min").and_then(|v| v.as_f64()),
            amount_max: r.matched.get("amount_max").and_then(|v| v.as_f64()),
            category_id: cat,
        });
    }
    out
}

fn rule_matches(r: &CompiledRule, norm: &str, amount: f64) -> bool {
    r.payees.iter().any(|p| p == norm)
        && r.amount_min.map_or(true, |m| amount >= m - 0.005)
        && r.amount_max.map_or(true, |m| amount <= m + 0.005)
}

fn cat_path(cache: &Cache, id: i64) -> String {
    cache.node_pos(id).map(|p| cache.categories[p as usize].path.clone()).unwrap_or_default()
}

/// Options for transaction `i`: the first matching rule (always), then model options when `with_model` and the
/// engine is enabled. At most three, model options at 0.30 confidence or better.
pub fn options(model: &Model, cache: &Cache, i: usize, with_model: bool) -> Vec<Opt> {
    let t = &cache.txns[i];
    let mut out = Vec::new();
    if let Some(r) = model.rules.iter().find(|r| rule_matches(r, &model.norms[i], t.amount)) {
        out.push(Opt { category_id: r.category_id, path: cat_path(cache, r.category_id), confidence: 1.0, source: "rule" });
    }
    if with_model {
        let s = model.suggest(cache);
        if s.enabled {
            for (cat, p) in s.predict(i) {
                if out.len() >= MAX_OPTIONS {
                    break;
                }
                if p < MIN_CONFIDENCE || out.iter().any(|o| o.category_id == cat) {
                    continue;
                }
                out.push(Opt {
                    category_id: cat,
                    path: cat_path(cache, cat),
                    confidence: (p * 1000.0).round() / 1000.0,
                    source: "model",
                });
            }
        }
    }
    out
}
