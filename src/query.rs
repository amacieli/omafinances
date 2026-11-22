//! Filter + sort + page over the in-memory `Cache`. Pure: no SQL, no I/O.
//!
//! `Query::parse` reads the URL query string (strict: bad or duplicate values are errors, never
//! ignored). `run` scans every transaction once, keeps the matches, and sorts only the page it needs
//! (`select_nth_unstable` then sort of the prefix), so `limit` small and offset early is cheap.
//!
//! Sort keys are integers precomputed in `Cache::build` (date ordinal, dense ranks for payee /
//! account / category path, order-preserving bits for amount), so the sort never compares strings.
//! Ties are always broken by id descending.

use crate::cache::{Cache, Txn};

pub const DEFAULT_LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Date,
    Amount,
    Payee,
    Account,
    Category,
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Money out: amount > 0.
    Out,
    /// Money in: amount < 0.
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryFilter {
    /// This category and all its descendants.
    Node(i64),
    /// `category=none`: no local category.
    Uncategorized,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub sort: Sort,
    pub desc: bool,
    pub account: Option<i64>,
    pub category: Option<CategoryFilter>,
    /// Inclusive date ordinals (`yyyymmdd`), see `date_key`.
    pub from: Option<u32>,
    pub to: Option<u32>,
    /// Bounds on the absolute amount.
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub flow: Option<Flow>,
    /// Lowercased, percent-decoded, non-empty substring.
    pub q: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            sort: Sort::Date,
            desc: true,
            account: None,
            category: None,
            from: None,
            to: None,
            min: None,
            max: None,
            flow: None,
            q: None,
            limit: DEFAULT_LIMIT,
            offset: 0,
        }
    }
}

pub struct QueryResult<'a> {
    /// Rows matching the filters (not just the page).
    pub total: usize,
    pub sum_out: f64,
    pub sum_in: f64,
    /// The requested page, in sort order.
    pub rows: Vec<&'a Txn>,
}

/// `YYYY-MM-DD` to `yyyymmdd`, only for a real calendar date. `None` otherwise.
pub fn date_key(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let mut v = 0u32;
        for &c in &b[r] {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as u32;
        }
        Some(v)
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    chrono::NaiveDate::from_ymd_opt(y as i32, m, d)?;
    Some(y * 10000 + m * 100 + d)
}

/// Strict integer: optional `-`, then digits. No `+`, no spaces.
fn parse_int(v: &str) -> Option<i64> {
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return None;
    }
    v.parse::<i64>().ok()
}

/// Strict decimal: optional `-`, digits, optional `.` and digits. Must be finite.
fn parse_decimal(v: &str) -> Option<f64> {
    let body = v.strip_prefix('-').unwrap_or(v);
    let has_dot = body.contains('.');
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || !digits(frac) {
        return None;
    }
    // at least one digit, and a dot must be followed by digits ("5." and "." are refused)
    if (int.is_empty() && frac.is_empty()) || (has_dot && frac.is_empty()) {
        return None;
    }
    v.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// Percent-decode one value. `+` stays a plus sign: spaces must be sent as `%20`.
pub(crate) fn percent_decode(s: &str) -> Result<String, ()> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b.get(i + 1..i + 3).ok_or(())?;
            let h = std::str::from_utf8(hex).map_err(|_| ())?;
            out.push(u8::from_str_radix(h, 16).map_err(|_| ())?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

const KNOWN: &[&str] = &["sort", "dir", "account", "category", "from", "to", "min", "max", "flow", "q", "limit", "offset"];

impl Query {
    /// Parse a raw query string (without the `?`). Errors name the parameter, never its value.
    pub fn parse(qs: &str) -> Result<Query, String> {
        let mut seen: Vec<&str> = Vec::new();
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for kv in qs.split('&').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            if !KNOWN.contains(&k) {
                return Err("unknown parameter".to_string());
            }
            if seen.contains(&k) {
                return Err(format!("duplicate parameter: {}", k));
            }
            seen.push(k);
            pairs.push((k, v));
        }
        let find = |key: &str| -> Option<&str> { pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v) };
        let decoded = |key: &str| -> Result<Option<String>, String> {
            match find(key) {
                None => Ok(None),
                Some(v) => percent_decode(v).map(Some).map_err(|_| format!("invalid value for {}", key)),
            }
        };

        let mut q = Query::default();

        if let Some(s) = decoded("sort")? {
            q.sort = match s.as_str() {
                "date" => Sort::Date,
                "amount" => Sort::Amount,
                "payee" => Sort::Payee,
                "account" => Sort::Account,
                "category" => Sort::Category,
                "notes" => Sort::Notes,
                _ => return Err("sort must be date, amount, payee, account, category or notes".to_string()),
            };
        }
        if let Some(d) = decoded("dir")? {
            q.desc = match d.as_str() {
                "asc" => false,
                "desc" => true,
                _ => return Err("dir must be asc or desc".to_string()),
            };
        }
        if let Some(v) = decoded("account")? {
            q.account = Some(parse_int(&v).ok_or("account must be an integer")?);
        }
        if let Some(v) = decoded("category")? {
            q.category = Some(if v == "none" {
                CategoryFilter::Uncategorized
            } else {
                CategoryFilter::Node(parse_int(&v).ok_or("category must be an integer or none")?)
            });
        }
        if let Some(v) = decoded("from")? {
            q.from = Some(date_key(&v).ok_or("from must be a date YYYY-MM-DD")?);
        }
        if let Some(v) = decoded("to")? {
            q.to = Some(date_key(&v).ok_or("to must be a date YYYY-MM-DD")?);
        }
        if let (Some(a), Some(b)) = (q.from, q.to) {
            if a > b {
                return Err("from is after to".to_string());
            }
        }
        if let Some(v) = decoded("min")? {
            q.min = Some(parse_decimal(&v).ok_or("min must be a number")?);
        }
        if let Some(v) = decoded("max")? {
            q.max = Some(parse_decimal(&v).ok_or("max must be a number")?);
        }
        if let (Some(a), Some(b)) = (q.min, q.max) {
            if a > b {
                return Err("min is greater than max".to_string());
            }
        }
        if let Some(v) = decoded("flow")? {
            q.flow = Some(match v.as_str() {
                "out" => Flow::Out,
                "in" => Flow::In,
                _ => return Err("flow must be out or in".to_string()),
            });
        }
        if let Some(v) = decoded("q")? {
            let v = v.to_lowercase();
            if !v.is_empty() {
                q.q = Some(v);
            }
        }
        if let Some(v) = decoded("limit")? {
            let n = parse_int(&v).ok_or("limit must be an integer from 1 to 500")?;
            if !(1..=MAX_LIMIT as i64).contains(&n) {
                return Err("limit must be an integer from 1 to 500".to_string());
            }
            q.limit = n as usize;
        }
        if let Some(v) = decoded("offset")? {
            let n = parse_int(&v).filter(|n| *n >= 0).ok_or("offset must be an integer of 0 or more")?;
            q.offset = n as usize;
        }
        Ok(q)
    }
}

/// Order-preserving map from f64 to u64 (total order; -0.0 sorts just below 0.0).
fn amount_key(x: f64) -> u64 {
    let b = x.to_bits();
    if b >> 63 == 1 {
        !b
    } else {
        b | (1u64 << 63)
    }
}

/// Resolved filter: category and account are decided once, not per row.
enum Cat {
    Any,
    Uncategorized,
    /// Preorder range `[lo, hi)` of the category and its descendants.
    Range(u32, u32),
    /// Unknown category id: nothing can match.
    Nothing,
}

struct Filter<'q> {
    q: &'q Query,
    cat: Cat,
}

impl Filter<'_> {
    fn matches(&self, t: &Txn) -> bool {
        let q = self.q;
        if let Some(a) = q.account {
            if t.account_id != Some(a) {
                return false;
            }
        }
        match self.cat {
            Cat::Any => {}
            Cat::Uncategorized => {
                if t.cat_pos.is_some() {
                    return false;
                }
            }
            Cat::Range(lo, hi) => match t.cat_pos {
                Some(p) if p >= lo && p < hi => {}
                _ => return false,
            },
            Cat::Nothing => return false,
        }
        if q.from.is_some() || q.to.is_some() {
            if t.date_key == 0 {
                return false;
            }
            if q.from.is_some_and(|f| t.date_key < f) || q.to.is_some_and(|to| t.date_key > to) {
                return false;
            }
        }
        match q.flow {
            Some(Flow::Out) if t.amount <= 0.0 => return false,
            Some(Flow::In) if t.amount >= 0.0 => return false,
            _ => {}
        }
        if q.min.is_some() || q.max.is_some() {
            let a = t.amount.abs();
            if q.min.is_some_and(|m| a < m) || q.max.is_some_and(|m| a > m) {
                return false;
            }
        }
        if let Some(s) = &q.q {
            if !(t.payee_lc.contains(s.as_str()) || t.original_lc.contains(s.as_str()) || t.notes_lc.contains(s.as_str()) || t.user_notes_lc.contains(s.as_str())) {
                return false;
            }
        }
        true
    }
}

fn sort_key(t: &Txn, s: Sort) -> u64 {
    match s {
        Sort::Date => t.date_key as u64,
        Sort::Amount => amount_key(t.amount),
        Sort::Payee => t.key_payee as u64,
        Sort::Account => t.key_account as u64,
        Sort::Category => t.key_category as u64,
        Sort::Notes => t.key_notes as u64,
    }
}

/// Filter, count, sum, sort and page. Sums cover every match, not only the page.
pub fn run<'a>(cache: &'a Cache, q: &Query) -> QueryResult<'a> {
    run_with(cache, q, None)
}

/// `run` with an extra predicate ANDed after the structured filters (the slash search).
pub fn run_with<'a>(cache: &'a Cache, q: &Query, extra: Option<&dyn Fn(&Txn) -> bool>) -> QueryResult<'a> {
    let cat = match q.category {
        None => Cat::Any,
        Some(CategoryFilter::Uncategorized) => Cat::Uncategorized,
        Some(CategoryFilter::Node(id)) => match cache.node_pos(id) {
            Some(p) => {
                let size = cache.categories[p as usize].size;
                Cat::Range(p, p + size)
            }
            None => Cat::Nothing,
        },
    };
    let f = Filter { q, cat };

    let mut sum_out = 0.0f64;
    let mut sum_in = 0.0f64;
    // (oriented key, id, index into cache.txns). Oriented: descending sorts invert the key.
    let mut hits: Vec<(u64, i64, u32)> = Vec::new();
    for (i, t) in cache.txns.iter().enumerate() {
        if !f.matches(t) || extra.is_some_and(|p| !p(t)) {
            continue;
        }
        if t.amount > 0.0 {
            sum_out += t.amount;
        } else if t.amount < 0.0 {
            sum_in -= t.amount;
        }
        let k = sort_key(t, q.sort);
        let k = if q.desc { !k } else { k };
        hits.push((k, t.id, i as u32));
    }
    let total = hits.len();

    let cmp = |a: &(u64, i64, u32), b: &(u64, i64, u32)| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1));
    let need = q.offset.saturating_add(q.limit);
    if need < hits.len() {
        hits.select_nth_unstable_by(need, cmp);
        hits.truncate(need);
    }
    hits.sort_unstable_by(cmp);

    let rows = hits
        .iter()
        .skip(q.offset)
        .take(q.limit)
        .map(|&(_, _, i)| &cache.txns[i as usize])
        .collect();
    QueryResult { total, sum_out, sum_in, rows }
}

#[cfg(test)]
mod tests;
