//! Compile a `Search` into a predicate over cache rows. Everything that can be decided once (category
//! subtree tables, cent values, is-kind masks) is decided in `compile`; `matches` does only integer
//! comparisons and substring tests against the lowercase haystacks the cache precomputes.

use super::*;
use crate::cache::{Cache, Txn, PATH_SEP};

enum Matcher {
    Text(String),
    Payee(Vec<String>),
    Account(Vec<String>),
    Lm(Vec<String>),
    Notes(Vec<String>),
    Id(Vec<i64>),
    /// Inclusive `yyyymmdd` bounds.
    Date(Vec<(u32, u32)>),
    Amount(Vec<CentSpec>),
    Is(Vec<IsKind>),
    /// `table[cat_pos]` says whether that category node matches; `none` is `c:none`.
    Cat { none: bool, table: Vec<bool> },
}

#[derive(Clone, Copy)]
enum CentSpec {
    Eq(i64),
    Gt(i64),
    Ge(i64),
    Lt(i64),
    Le(i64),
    Range(Option<i64>, Option<i64>),
}

fn cents(x: f64) -> i64 {
    (x * 100.0).round() as i64
}

impl CentSpec {
    fn from(a: &AmountSpec) -> CentSpec {
        match *a {
            AmountSpec::Eq(v) => CentSpec::Eq(cents(v)),
            AmountSpec::Gt(v) => CentSpec::Gt(cents(v)),
            AmountSpec::Ge(v) => CentSpec::Ge(cents(v)),
            AmountSpec::Lt(v) => CentSpec::Lt(cents(v)),
            AmountSpec::Le(v) => CentSpec::Le(cents(v)),
            AmountSpec::Range(l, h) => CentSpec::Range(l.map(cents), h.map(cents)),
        }
    }

    fn test(self, c: i64) -> bool {
        match self {
            CentSpec::Eq(v) => c == v,
            CentSpec::Gt(v) => c > v,
            CentSpec::Ge(v) => c >= v,
            CentSpec::Lt(v) => c < v,
            CentSpec::Le(v) => c <= v,
            CentSpec::Range(l, h) => l.is_none_or(|l| c >= l) && h.is_none_or(|h| c <= h),
        }
    }
}

struct Pred {
    neg: bool,
    m: Matcher,
}

/// A search compiled against one cache. Valid only for that cache (category tables are positional).
pub struct Compiled {
    preds: Vec<Pred>,
}

impl Compiled {
    pub fn is_empty(&self) -> bool {
        self.preds.is_empty()
    }

    /// All terms hold for this row.
    pub fn matches(&self, t: &Txn) -> bool {
        self.preds.iter().all(|p| p.m.test(t) != p.neg)
    }
}

impl Matcher {
    /// Rough evaluation cost, cheapest first.
    fn cost(&self) -> u8 {
        match self {
            Matcher::Id(_) => 0,
            Matcher::Is(_) => 1,
            Matcher::Date(_) => 2,
            Matcher::Amount(_) => 3,
            Matcher::Cat { .. } => 4,
            Matcher::Account(_) | Matcher::Lm(_) => 5,
            Matcher::Payee(_) | Matcher::Notes(_) => 6,
            Matcher::Text(_) => 7,
        }
    }

    fn test(&self, t: &Txn) -> bool {
        match self {
            Matcher::Text(s) => t.payee_lc.contains(s.as_str()) || t.original_lc.contains(s.as_str()) || t.notes_lc.contains(s.as_str()) || t.user_notes_lc.contains(s.as_str()),
            Matcher::Payee(v) => v.iter().any(|s| t.payee_lc.contains(s.as_str()) || t.original_lc.contains(s.as_str())),
            Matcher::Account(v) => v.iter().any(|s| t.account_lc.contains(s.as_str())),
            Matcher::Lm(v) => v.iter().any(|s| t.lm_lc.contains(s.as_str())),
            // `n:*` is "has any note"; otherwise a substring of the user's note or Lunch Money's.
            Matcher::Notes(v) => v.iter().any(|s| {
                if s == "*" {
                    !t.user_notes_lc.is_empty() || !t.notes_lc.is_empty()
                } else {
                    t.user_notes_lc.contains(s.as_str()) || t.notes_lc.contains(s.as_str())
                }
            }),
            Matcher::Id(v) => v.contains(&t.id),
            Matcher::Date(v) => t.date_key != 0 && v.iter().any(|&(lo, hi)| t.date_key >= lo && t.date_key <= hi),
            Matcher::Amount(v) => {
                let c = cents(t.amount.abs());
                v.iter().any(|a| a.test(c))
            }
            Matcher::Is(v) => v.iter().any(|k| match k {
                IsKind::Recurring => t.recurring,
                IsKind::Pending => t.pending,
                IsKind::Transfer => t.lm_lc.contains("transfer"),
                IsKind::Income => t.amount < 0.0,
                IsKind::Expense => t.amount > 0.0,
                IsKind::Uncat => t.cat_pos.is_none(),
                IsKind::Categorized => t.cat_pos.is_some(),
            }),
            Matcher::Cat { none, table } => match t.cat_pos {
                None => *none,
                Some(p) => table.get(p as usize).copied().unwrap_or(false),
            },
        }
    }
}

pub fn compile(search: &Search, cache: &Cache) -> Compiled {
    let mut preds: Vec<Pred> = search
        .terms
        .iter()
        .map(|term| {
            let m = match &term.kind {
                TermKind::Text(s) => Matcher::Text(s.clone()),
                TermKind::Payee(v) => Matcher::Payee(v.clone()),
                TermKind::Account(v) => Matcher::Account(v.clone()),
                TermKind::LmCategory(v) => Matcher::Lm(v.clone()),
                TermKind::Notes(v) => Matcher::Notes(v.clone()),
                TermKind::Id(v) => Matcher::Id(v.clone()),
                TermKind::Date(v) => Matcher::Date(v.iter().map(|d| (d.lo.unwrap_or(0), d.hi.unwrap_or(u32::MAX))).collect()),
                TermKind::Amount(v) => Matcher::Amount(v.iter().map(CentSpec::from).collect()),
                TermKind::Is(v) => Matcher::Is(v.clone()),
                TermKind::Category(v) => category_matcher(v, cache),
            };
            Pred { neg: term.neg, m }
        })
        .collect();
    preds.sort_by_key(|p| p.m.cost()); // stable: equal costs keep the typed order
    Compiled { preds }
}

fn category_matcher(specs: &[CatSpec], cache: &Cache) -> Matcher {
    let none = specs.iter().any(|s| matches!(s, CatSpec::None));
    let paths: Vec<&Vec<String>> = specs
        .iter()
        .filter_map(|s| match s {
            CatSpec::Path(p) => Some(p),
            CatSpec::None => None,
        })
        .collect();
    let table = cache
        .categories
        .iter()
        .map(|node| {
            let segs: Vec<String> = node.path.split(PATH_SEP).map(|s| s.to_lowercase()).collect();
            paths.iter().any(|p| segs.len() >= p.len() && p.iter().zip(&segs).all(|(want, have)| have.starts_with(want.as_str())))
        })
        .collect();
    Matcher::Cat { none, table }
}
