//! Completions for the token under the cursor. Works on text that does not parse (the user is mid-word).
//!
//! The reply names the byte range to replace (`start..end`) and each item's `insert` is the full
//! replacement for that range, so the UI does `text[..start] + insert + text[end..]`. The range starts
//! at the token start (including a leading `-`) and ends at the end of the current comma item, or of
//! the token when a field name is being completed. Only the text before the cursor is matched.

use super::parse::{preset_range, split, PRESETS};
use super::*;
use crate::cache::Cache;
use chrono::NaiveDate;

pub const MAX_ITEMS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub insert: String,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completions {
    pub start: usize,
    pub end: usize,
    pub items: Vec<Completion>,
}

impl Completions {
    pub fn to_json(&self) -> serde_json::Value {
        let items: Vec<_> = self.items.iter().map(|i| serde_json::json!({ "insert": i.insert, "label": i.label, "detail": i.detail })).collect();
        serde_json::json!({ "start": self.start, "end": self.end, "items": items })
    }
}

const FIELD_ITEMS: &[(&str, &str, &[&str])] = &[
    ("p:", "payee", &["p", "payee"]),
    ("a:", "account", &["a", "acct", "account"]),
    ("c:", "category", &["c", "cat", "category"]),
    ("d:", "date", &["d", "date"]),
    ("amount:", "amount (absolute)", &["amount", "amt"]),
    ("is:", "flag: recurring, pending, income, ...", &["is"]),
    ("sort:", "sort order", &["sort"]),
    ("n:", "notes", &["n", "note", "notes"]),
    ("lm:", "Lunch Money category", &["lm"]),
];

fn txns(n: usize) -> String {
    format!("{} txn{}", n, if n == 1 { "" } else { "s" })
}

pub fn complete(cache: Option<&Cache>, input: &str, pos: usize, today: NaiveDate) -> Completions {
    let mut pos = pos.min(input.len());
    while !input.is_char_boundary(pos) {
        pos -= 1;
    }
    let (toks, _) = split(input);
    let (start, tok_end) = toks.iter().find(|t| t.start <= pos && pos <= t.end).map_or((pos, pos), |t| (t.start, t.end));
    let before = &input[start..pos];
    let neg = if before.starts_with('-') { "-" } else { "" };
    let rest = &before[neg.len()..];

    // Walk the typed text: quote state, the first unquoted colon, the last unquoted comma after it.
    let (mut inq, mut colon, mut comma) = (false, None::<usize>, None::<usize>);
    let mut it = rest.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '\\' if matches!(it.peek(), Some(&(_, '"')) | Some(&(_, '\\'))) => {
                it.next();
            }
            '"' => inq = !inq,
            ':' if !inq && colon.is_none() => colon = Some(i),
            ',' if !inq && colon.is_some() => comma = Some(i),
            _ => {}
        }
    }

    let Some(ci) = colon else {
        // A field name (or free text, which has no completions).
        let typed = rest.to_lowercase();
        let items = if rest.contains('"') { Vec::new() } else { field_items(&typed, neg) };
        return Completions { start, end: tok_end, items };
    };

    // End of the current item: continue the scan after the cursor to the next unquoted comma.
    let mut end = tok_end;
    let mut q = inq;
    let mut it = input[pos..tok_end].char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '\\' if matches!(it.peek(), Some(&(_, '"')) | Some(&(_, '\\'))) => {
                it.next();
            }
            '"' => q = !q,
            ',' if !q => {
                end = pos + i;
                break;
            }
            _ => {}
        }
    }

    let field = Field::from_name(&rest[..ci].to_lowercase());
    let item_from = comma.unwrap_or(ci) + 1;
    let head = format!("{}{}", neg, &rest[..item_from]);
    let typed_raw = &rest[item_from..];
    let typed: String = typed_raw.chars().filter(|c| *c != '"').collect::<String>().to_lowercase();
    let wrap = |value: &str, label: &str, detail: String| Completion { insert: format!("{}{}", head, quote(value, false)), label: label.to_string(), detail };

    let items = match (field, cache) {
        (Some(Field::Date), _) => date_items(&typed, today, &head),
        (Some(Field::Is), c) => is_items(&typed, c, &head),
        (Some(Field::Sort), _) => sort_items(&typed, &head),
        (Some(Field::Payee), Some(c)) => payee_items(c, &typed, &wrap),
        (Some(Field::Account), Some(c)) => account_items(c, &typed, &wrap),
        (Some(Field::Category), Some(c)) => category_items(c, &typed, &wrap),
        _ => Vec::new(),
    };
    Completions { start, end, items }
}

fn field_items(typed: &str, neg: &str) -> Vec<Completion> {
    let mut hits: Vec<(u8, usize, &(&str, &str, &[&str]))> = Vec::new();
    for (i, f) in FIELD_ITEMS.iter().enumerate() {
        if let Some(rank) = f.2.iter().position(|n| n.starts_with(typed)).map(|_| if f.2[0] == typed { 0 } else if f.2[0].starts_with(typed) { 1 } else { 2 }) {
            hits.push((rank, i, f));
        }
    }
    hits.sort_by_key(|h| (h.0, h.1));
    hits.into_iter()
        .map(|(_, _, f)| Completion { insert: format!("{}{}", neg, f.0), label: f.0.to_string(), detail: f.1.to_string() })
        .take(MAX_ITEMS)
        .collect()
}

fn payee_items(cache: &Cache, typed: &str, wrap: &dyn Fn(&str, &str, String) -> Completion) -> Vec<Completion> {
    // `payee_freq` is already most-frequent-first, so the first hits of each class are the best.
    let (mut pre, mut sub): (Vec<&(String, usize)>, Vec<&(String, usize)>) = (Vec::new(), Vec::new());
    for e in &cache.payee_freq {
        if pre.len() >= MAX_ITEMS {
            break;
        }
        let lc = e.0.to_lowercase();
        if lc.starts_with(typed) {
            pre.push(e);
        } else if sub.len() < MAX_ITEMS && lc.contains(typed) {
            sub.push(e);
        }
    }
    pre.into_iter().chain(sub).take(MAX_ITEMS).map(|(p, n)| wrap(p, p, txns(*n))).collect()
}

fn account_items(cache: &Cache, typed: &str, wrap: &dyn Fn(&str, &str, String) -> Completion) -> Vec<Completion> {
    let mut hits: Vec<(u8, &crate::cache::AccountRow)> = cache
        .accounts
        .iter()
        .filter_map(|a| {
            let lc = a.name.to_lowercase();
            if lc.starts_with(typed) {
                Some((0, a))
            } else if lc.contains(typed) {
                Some((1, a))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by(|x, y| x.0.cmp(&y.0).then(y.1.count.cmp(&x.1.count)).then_with(|| x.1.name.cmp(&y.1.name)));
    hits.into_iter().take(MAX_ITEMS).map(|(_, a)| wrap(&a.name, &a.name, txns(a.count))).collect()
}

fn category_items(cache: &Cache, typed: &str, wrap: &dyn Fn(&str, &str, String) -> Completion) -> Vec<Completion> {
    // (rank, depth, -subtree count, path) so shallow, busy categories come first.
    let mut hits: Vec<(u8, u32, std::cmp::Reverse<usize>, String, String, usize)> = Vec::new();
    if "none".starts_with(typed) {
        hits.push((0, 0, std::cmp::Reverse(cache.uncategorized), "none".to_string(), "none".to_string(), cache.uncategorized));
    }
    for node in &cache.categories {
        let segs: Vec<&str> = node.path.split(crate::cache::PATH_SEP).collect();
        let joined = segs.join("/");
        let lc = joined.to_lowercase();
        let rank = if lc.starts_with(typed) {
            0
        } else if !typed.contains('/') && segs.iter().any(|s| s.to_lowercase().starts_with(typed)) {
            1
        } else if lc.contains(typed) {
            2
        } else {
            continue;
        };
        hits.push((rank, node.depth, std::cmp::Reverse(node.count_subtree), joined.clone(), joined, node.count_subtree));
    }
    hits.sort_by(|a, b| (a.0, a.1, a.2, &a.3).cmp(&(b.0, b.1, b.2, &b.3)));
    hits.into_iter().take(MAX_ITEMS).map(|h| wrap(&h.4, &h.4, txns(h.5))).collect()
}

fn date_items(typed: &str, today: NaiveDate, head: &str) -> Vec<Completion> {
    PRESETS
        .iter()
        .filter(|p| p.starts_with(typed))
        .map(|p| {
            let detail = match preset_range(p, today) {
                Some((a, b)) if a == b => a.format("%Y-%m-%d").to_string(),
                Some((a, b)) => format!("{}..{}", a.format("%Y-%m-%d"), b.format("%Y-%m-%d")),
                None => String::new(),
            };
            Completion { insert: format!("{}{}", head, p), label: p.to_string(), detail }
        })
        .take(MAX_ITEMS)
        .collect()
}

fn is_items(typed: &str, cache: Option<&Cache>, head: &str) -> Vec<Completion> {
    let mut counts = [0usize; 7];
    if let Some(c) = cache {
        for t in &c.txns {
            let flags = [t.recurring, t.pending, t.lm_lc.contains("transfer"), t.amount < 0.0, t.amount > 0.0, t.cat_pos.is_none(), t.cat_pos.is_some()];
            for (n, f) in counts.iter_mut().zip(flags) {
                *n += usize::from(f);
            }
        }
    }
    IsKind::ALL
        .iter()
        .enumerate()
        .filter(|(_, k)| k.name().starts_with(typed))
        .map(|(i, k)| Completion {
            insert: format!("{}{}", head, k.name()),
            label: k.name().to_string(),
            detail: if cache.is_some() { txns(counts[i]) } else { String::new() },
        })
        .collect()
}

fn sort_items(typed: &str, head: &str) -> Vec<Completion> {
    let (desc_only, key) = match typed.strip_prefix('-') {
        Some(k) => (true, k),
        None => (false, typed),
    };
    let mut out = Vec::new();
    for desc in [false, true] {
        if desc_only && !desc {
            continue;
        }
        for k in SortKey::ALL.iter().filter(|k| k.name().starts_with(key)) {
            let v = sort_text(*k, if desc { Dir::Desc } else { Dir::Asc });
            out.push(Completion { insert: format!("{}{}", head, v), label: v, detail: if desc { "descending" } else { "ascending" }.to_string() });
        }
    }
    out.truncate(MAX_ITEMS);
    out
}
