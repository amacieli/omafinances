//! The whole mirror in memory, built once at startup and after `POST /refresh`.
//!
//! `Cache::build` is pure (a `Snapshot` in, no I/O), so the perf test can build 50k rows without a
//! database. Everything the query engine sorts or filters on is precomputed here: lowercased search
//! text, the date ordinal, dense ranks for payee / account / category path, and each transaction's
//! position in the category tree (so "this category and its descendants" is one range check).

use crate::db::{Db, RawAccount, RawCategory, RawTxn, Snapshot};
use crate::query::date_key;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Separator in category paths: `Home › Improvement › Paint`.
pub const PATH_SEP: &str = " › ";

#[derive(Debug, Clone)]
pub struct Txn {
    pub id: i64,
    pub date: String,
    /// `yyyymmdd`, 0 when the stored date is not a valid calendar date.
    pub date_key: u32,
    /// Empty string when the payee is NULL (the list has always shown it that way).
    pub payee: String,
    pub original_name: Option<String>,
    pub notes: Option<String>,
    pub amount: f64,
    /// Empty string when the currency is NULL.
    pub currency: String,
    pub account_id: Option<i64>,
    /// Display name of the account, as the list has always shown it.
    pub account: Option<String>,
    /// Local category id (always a known tree node, else `None`).
    pub category_id: Option<i64>,
    pub lm_category: Option<String>,
    pub pending: bool,
    pub recurring: bool,
    pub status: Option<String>,
    pub source: Option<String>,
    /// The stored `tag_ids` JSON, when it is an array; `[]` otherwise.
    pub tags: Value,
    pub recurring_id: Option<i64>,
    pub external_id: Option<String>,
    pub created_at_lm: Option<String>,
    pub updated_at_lm: Option<String>,

    /// Position of the local category in `Cache::categories` (preorder), if any.
    pub cat_pos: Option<u32>,
    pub payee_lc: String,
    pub original_lc: String,
    pub notes_lc: String,
    /// Lowercased account display name and Lunch Money category name ("" when absent).
    pub account_lc: String,
    pub lm_lc: String,
    /// Dense ranks (1-based; 0 = none) used as integer sort keys.
    pub key_payee: u32,
    pub key_account: u32,
    pub key_category: u32,
}

#[derive(Debug, Clone)]
pub struct CatNode {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub kind: String,
    /// Stored sibling order (`category.sort`); the display order is parent, then siblings by sort and name.
    pub sort: i64,
    pub depth: u32,
    pub path: String,
    /// Number of nodes in this subtree, self included. Descendants are `pos+1 .. pos+size`.
    pub size: u32,
    /// Rank of `path` among all node paths (1-based), for sorting transactions by category.
    pub rank: u32,
    /// Transactions whose local category is exactly this node.
    pub count: usize,
    /// ... and all its descendants.
    pub count_subtree: usize,
    /// Sum of positive amounts (money out) over the subtree.
    pub sum_out_subtree: f64,
}

#[derive(Debug, Clone)]
pub struct AccountRow {
    pub id: i64,
    /// Display name if set, else the account name.
    pub name: String,
    pub kind: Option<String>,
    pub institution: Option<String>,
    pub mask: Option<String>,
    pub currency: Option<String>,
    /// Latest balance snapshot; `None` when there is none.
    pub balance: Option<f64>,
    pub count: usize,
    pub sum_out: f64,
}

pub struct Cache {
    /// All transactions, ordered by id.
    pub txns: Vec<Txn>,
    /// Ordered by name (case-insensitive), then id.
    pub accounts: Vec<AccountRow>,
    /// Category tree in display order: parent before children, siblings by `sort` then name.
    pub categories: Vec<CatNode>,
    /// Transactions with no local category.
    pub uncategorized: usize,
    /// Distinct non-empty payees with their transaction counts, most frequent first (ties by name).
    pub payee_freq: Vec<(String, usize)>,
    pos_by_id: HashMap<i64, u32>,
}

impl Cache {
    pub fn load(db: &Db) -> Result<Cache> {
        Ok(Cache::build(db.load_all()?))
    }

    pub fn build(snap: Snapshot) -> Cache {
        let mut categories = build_tree(&snap.categories);
        let pos_by_id: HashMap<i64, u32> = categories.iter().enumerate().map(|(i, c)| (c.id, i as u32)).collect();
        let n = categories.len();

        let mut txns: Vec<Txn> = snap.txns.into_iter().map(|r| convert(r, &pos_by_id)).collect();
        // `api` looks transactions up by id with a binary search, so the order is part of the contract.
        txns.sort_unstable_by_key(|t| t.id);

        // Dense ranks for the three text sort keys.
        let payee_rank = dense_ranks(txns.iter().map(|t| t.payee_lc.as_str()));
        let account_rank = dense_ranks(txns.iter().filter_map(|t| t.account.as_deref()));
        let kp: Vec<u32> = txns.iter().map(|t| payee_rank[t.payee_lc.as_str()]).collect();
        let ka: Vec<u32> = txns
            .iter()
            .map(|t| t.account.as_deref().map_or(0, |a| account_rank[a]))
            .collect();
        let path_rank = dense_ranks(categories.iter().map(|c| c.path.as_str()));
        let kc: Vec<u32> = categories.iter().map(|c| path_rank[c.path.as_str()]).collect();
        for (c, r) in categories.iter_mut().zip(kc) {
            c.rank = r;
        }
        for (i, t) in txns.iter_mut().enumerate() {
            t.key_payee = kp[i];
            t.key_account = ka[i];
            t.key_category = t.cat_pos.map_or(0, |p| categories[p as usize].rank);
        }

        // Per-category totals, then prefix sums over preorder so each subtree is a range difference.
        let mut direct_count = vec![0usize; n];
        let mut direct_out = vec![0f64; n];
        let mut by_account: HashMap<i64, (usize, f64)> = HashMap::new();
        let mut uncategorized = 0usize;
        for t in &txns {
            let out = if t.amount > 0.0 { t.amount } else { 0.0 };
            match t.cat_pos {
                Some(p) => {
                    direct_count[p as usize] += 1;
                    direct_out[p as usize] += out;
                }
                None => uncategorized += 1,
            }
            if let Some(a) = t.account_id {
                let e = by_account.entry(a).or_insert((0, 0.0));
                e.0 += 1;
                e.1 += out;
            }
        }
        let mut pc = vec![0usize; n + 1];
        let mut ps = vec![0f64; n + 1];
        for i in 0..n {
            pc[i + 1] = pc[i] + direct_count[i];
            ps[i + 1] = ps[i] + direct_out[i];
        }
        for (i, c) in categories.iter_mut().enumerate() {
            let end = i + c.size as usize;
            c.count = direct_count[i];
            c.count_subtree = pc[end] - pc[i];
            c.sum_out_subtree = ps[end] - ps[i];
        }

        let balances: HashMap<i64, f64> = snap.balances.into_iter().collect();
        let mut accounts: Vec<AccountRow> = snap
            .accounts
            .into_iter()
            .map(|a| account_row(a, &by_account, &balances))
            .collect();
        accounts.sort_by(|x, y| (x.name.to_lowercase(), x.id).cmp(&(y.name.to_lowercase(), y.id)));

        let mut freq: HashMap<&str, usize> = HashMap::new();
        for t in &txns {
            if !t.payee.is_empty() {
                *freq.entry(t.payee.as_str()).or_insert(0) += 1;
            }
        }
        let mut payee_freq: Vec<(String, usize)> = freq.into_iter().map(|(p, n)| (p.to_string(), n)).collect();
        payee_freq.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Cache { txns, accounts, categories, uncategorized, payee_freq, pos_by_id }
    }

    /// Preorder position of a category id, if it is in the tree.
    pub fn node_pos(&self, id: i64) -> Option<u32> {
        self.pos_by_id.get(&id).copied()
    }

    /// Category path of a transaction, or `None`.
    pub fn category_path(&self, t: &Txn) -> Option<&str> {
        t.cat_pos.map(|p| self.categories[p as usize].path.as_str())
    }
}

fn account_row(a: RawAccount, by_account: &HashMap<i64, (usize, f64)>, balances: &HashMap<i64, f64>) -> AccountRow {
    let (count, sum_out) = by_account.get(&a.id).copied().unwrap_or((0, 0.0));
    AccountRow {
        id: a.id,
        name: a.display_name.filter(|s| !s.is_empty()).unwrap_or(a.name),
        kind: a.kind,
        institution: a.institution,
        mask: a.mask,
        currency: a.currency,
        balance: balances.get(&a.id).copied(),
        count,
        sum_out,
    }
}

fn convert(r: RawTxn, pos_by_id: &HashMap<i64, u32>) -> Txn {
    let cat_pos = r.category_local_id.and_then(|id| pos_by_id.get(&id).copied());
    let payee = r.payee.unwrap_or_default();
    let tags = r
        .tag_ids
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([]));
    // A transaction belongs to a Lunch Money recurring series when it carries a recurring_id.
    let recurring = r.recurring_id.is_some();
    Txn {
        id: r.id,
        date_key: date_key(&r.date).unwrap_or(0),
        date: r.date,
        payee_lc: payee.to_lowercase(),
        payee,
        original_lc: r.original_name.as_deref().unwrap_or("").to_lowercase(),
        original_name: r.original_name,
        notes_lc: r.notes.as_deref().unwrap_or("").to_lowercase(),
        notes: r.notes,
        account_lc: r.account.as_deref().unwrap_or("").to_lowercase(),
        lm_lc: r.lm_category.as_deref().unwrap_or("").to_lowercase(),
        amount: r.amount,
        currency: r.currency.unwrap_or_default(),
        account_id: r.account_id,
        account: r.account,
        category_id: cat_pos.map(|_| r.category_local_id.unwrap_or_default()),
        lm_category: r.lm_category,
        pending: r.is_pending,
        recurring,
        status: r.status,
        source: r.source,
        tags,
        recurring_id: r.recurring_id,
        external_id: r.external_id,
        created_at_lm: r.created_at_lm,
        updated_at_lm: r.updated_at_lm,
        cat_pos,
        key_payee: 0,
        key_account: 0,
        key_category: 0,
    }
}

/// 1-based rank of each distinct string in sorted order.
fn dense_ranks<'a, I: Iterator<Item = &'a str>>(vals: I) -> HashMap<&'a str, u32> {
    let set: BTreeSet<&str> = vals.collect();
    set.into_iter().enumerate().map(|(i, s)| (s, i as u32 + 1)).collect()
}

/// Build the display-ordered tree with subtree sizes. A node whose parent is missing, or that sits on
/// a parent cycle, is placed at the top level, so every row appears exactly once.
fn build_tree(raw: &[RawCategory]) -> Vec<CatNode> {
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| (raw[a].sort, &raw[a].name, raw[a].id).cmp(&(raw[b].sort, &raw[b].name, raw[b].id)));
    let ids: HashSet<i64> = raw.iter().map(|c| c.id).collect();
    let mut children: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for &i in &order {
        match raw[i].parent_id {
            Some(p) if p != raw[i].id && ids.contains(&p) => children.entry(p).or_default().push(i),
            _ => roots.push(i),
        }
    }

    struct Ctx<'a> {
        raw: &'a [RawCategory],
        children: &'a HashMap<i64, Vec<usize>>,
        seen: Vec<bool>,
        out: Vec<CatNode>,
    }
    fn visit(cx: &mut Ctx, i: usize, depth: u32, parent_path: Option<&str>) {
        cx.seen[i] = true;
        let r = &cx.raw[i];
        let path = match parent_path {
            Some(p) => format!("{}{}{}", p, PATH_SEP, r.name),
            None => r.name.clone(),
        };
        let start = cx.out.len();
        cx.out.push(CatNode {
            id: r.id,
            parent_id: r.parent_id,
            name: r.name.clone(),
            kind: r.kind.clone(),
            sort: r.sort,
            depth,
            path: path.clone(),
            size: 0,
            rank: 0,
            count: 0,
            count_subtree: 0,
            sum_out_subtree: 0.0,
        });
        let kids: Vec<usize> = cx.children.get(&r.id).cloned().unwrap_or_default();
        for k in kids {
            if !cx.seen[k] {
                visit(cx, k, depth + 1, Some(&path));
            }
        }
        cx.out[start].size = (cx.out.len() - start) as u32;
    }

    let mut cx = Ctx { raw, children: &children, seen: vec![false; raw.len()], out: Vec::with_capacity(raw.len()) };
    for &r in &roots {
        if !cx.seen[r] {
            visit(&mut cx, r, 0, None);
        }
    }
    for &i in &order {
        if !cx.seen[i] {
            visit(&mut cx, i, 0, None);
        }
    }
    cx.out
}

#[cfg(test)]
pub(crate) mod testing {
    //! Synthetic data shared by the query, cache and API tests. Deterministic (LCG), no secrets.
    use super::*;

    /// A small three-level tree plus `n` transactions over two years, five accounts, ~10% uncategorized.
    pub(crate) fn synthetic(n: usize) -> Snapshot {
        let mut cats = Vec::new();
        let mut id = 1i64;
        for (mi, main) in ["Home", "Salary", "Travel"].iter().enumerate() {
            let m = id;
            cats.push(RawCategory { id, parent_id: None, name: main.to_string(), kind: if mi == 1 { "income".into() } else { "expense".into() }, sort: mi as i64 });
            id += 1;
            for s in 0..3 {
                let sub = id;
                cats.push(RawCategory { id, parent_id: Some(m), name: format!("Sub {}", s), kind: "expense".into(), sort: (2 - s) as i64 });
                id += 1;
                for ss in 0..3 {
                    cats.push(RawCategory { id, parent_id: Some(sub), name: format!("Leaf {}", ss), kind: "expense".into(), sort: 0 });
                    id += 1;
                }
            }
        }
        let accounts: Vec<RawAccount> = (1..=5)
            .map(|a| RawAccount {
                id: a,
                name: format!("acct {}", a),
                display_name: if a % 2 == 0 { Some(format!("Account {}", 6 - a)) } else { None },
                kind: Some("depository".into()),
                institution: Some("Bank".into()),
                mask: Some(format!("{:04}", a)),
                currency: Some("USD".into()),
            })
            .collect();
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut rnd = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let payees = [
            "Amazon", "amazon mktplace", "Whole Foods", "Shell", "Netflix", "Rent", "Payroll", "Costco", "Delta Air",
            "Hardware Store", "Coffee", "Electric Co",
        ];
        let leaf_ids: Vec<i64> = cats.iter().filter(|c| c.parent_id.is_some() && cats.iter().any(|d| Some(d.id) == c.parent_id && d.parent_id.is_some())).map(|c| c.id).collect();
        let mut txns = Vec::with_capacity(n);
        for i in 0..n {
            let day = rnd() % 730;
            let date = (base + chrono::Days::new(day as u64)).format("%Y-%m-%d").to_string();
            let cents = (rnd() % 40_000) as f64 / 100.0;
            let sign = if rnd() % 5 == 0 { -1.0 } else { 1.0 };
            let payee = payees[(rnd() as usize) % payees.len()];
            let cat = if rnd() % 10 == 0 { None } else { Some(leaf_ids[(rnd() as usize) % leaf_ids.len()]) };
            let acct = 1 + (rnd() % 5) as i64;
            txns.push(RawTxn {
                id: i as i64 + 1,
                date,
                payee: Some(format!("{} {}", payee, i % 50)),
                original_name: Some(format!("ORIG {}", i % 7)),
                notes: if i % 3 == 0 { Some(format!("note {}", i % 11)) } else { None },
                amount: sign * cents,
                currency: Some("USD".into()),
                status: Some("cleared".into()),
                source: Some("plaid".into()),
                tag_ids: None,
                recurring_id: None,
                external_id: None,
                created_at_lm: None,
                updated_at_lm: None,
                is_pending: i % 97 == 0,
                category_id: None,
                category_local_id: cat,
                lm_category: None,
                account_id: Some(acct),
                account: Some(accounts[(acct - 1) as usize].display_name.clone().unwrap_or_else(|| accounts[(acct - 1) as usize].name.clone())),
            });
        }
        Snapshot { txns, accounts, categories: cats, balances: vec![(1, 100.0)] }
    }
}

#[cfg(test)]
mod tests;
