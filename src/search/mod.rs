//! Slash search: a small local query language (`p:amazon c:home d:this-month >50 sort:-amount`),
//! parsed in Rust and evaluated against the in-memory `Cache`.
//!
//! * `parse` turns the text into a `Search` (an AST of ANDed `Term`s plus an optional sort directive).
//!   Relative dates (`this-month`, `30d`) are resolved against the `today` the caller supplies, so the
//!   AST holds concrete date ranges and tests are deterministic. The original spelling of each date
//!   value is kept so `canonical()` stays readable and re-parses to an equal AST.
//! * `Search::compile` builds a predicate over cache rows (`eval`). It composes with the structured
//!   filters via `query::run_with`: final = structured filters AND search.
//! * `complete` suggests tokens for the cursor position (`complete`).
//! * `history` stores the recent query texts in the config dir.
//!
//! Money convention: amount POSITIVE = money out. `amount:` terms compare the absolute amount.
//! Nothing here logs: query text is private.

pub mod complete;
pub mod eval;
pub mod history;
pub mod parse;

pub use eval::Compiled;
pub use parse::{parse, ParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Date,
    Amount,
    Payee,
    Account,
    Category,
    Notes,
}

impl SortKey {
    pub const ALL: [SortKey; 6] = [SortKey::Date, SortKey::Amount, SortKey::Payee, SortKey::Account, SortKey::Category, SortKey::Notes];

    pub fn name(self) -> &'static str {
        match self {
            SortKey::Date => "date",
            SortKey::Amount => "amount",
            SortKey::Payee => "payee",
            SortKey::Account => "account",
            SortKey::Category => "category",
            SortKey::Notes => "notes",
        }
    }

    pub fn from_name(s: &str) -> Option<SortKey> {
        SortKey::ALL.into_iter().find(|k| k.name() == s)
    }

    pub fn to_query(self) -> crate::query::Sort {
        use crate::query::Sort;
        match self {
            SortKey::Date => Sort::Date,
            SortKey::Amount => Sort::Amount,
            SortKey::Payee => Sort::Payee,
            SortKey::Account => Sort::Account,
            SortKey::Category => Sort::Category,
            SortKey::Notes => Sort::Notes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

/// `sort:amount` / `sort:-amount`.
pub fn sort_text(key: SortKey, dir: Dir) -> String {
    format!("{}{}", if dir == Dir::Desc { "-" } else { "" }, key.name())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Search {
    pub terms: Vec<Term>,
    pub sort: Option<(SortKey, Dir)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub neg: bool,
    pub kind: TermKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TermKind {
    /// Bare word or quoted phrase (lowercased): substring over payee, original name and notes (Lunch Money's and the user's).
    Text(String),
    /// Every list is an OR over its items (lowercased substrings, ids, ...).
    Payee(Vec<String>),
    Account(Vec<String>),
    Category(Vec<CatSpec>),
    LmCategory(Vec<String>),
    Notes(Vec<String>),
    Id(Vec<i64>),
    Date(Vec<DateSpec>),
    Amount(Vec<AmountSpec>),
    Is(Vec<IsKind>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CatSpec {
    /// `c:none`: no local category.
    None,
    /// Lowercased path segments from the root; each segment prefix-matches the node's segment.
    Path(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsKind {
    Recurring,
    Pending,
    Transfer,
    Income,
    Expense,
    Uncat,
    Categorized,
}

impl IsKind {
    pub const ALL: [IsKind; 7] =
        [IsKind::Recurring, IsKind::Pending, IsKind::Transfer, IsKind::Income, IsKind::Expense, IsKind::Uncat, IsKind::Categorized];

    pub fn name(self) -> &'static str {
        match self {
            IsKind::Recurring => "recurring",
            IsKind::Pending => "pending",
            IsKind::Transfer => "transfer",
            IsKind::Income => "income",
            IsKind::Expense => "expense",
            IsKind::Uncat => "uncat",
            IsKind::Categorized => "categorized",
        }
    }

    pub fn from_name(s: &str) -> Option<IsKind> {
        IsKind::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// A condition on the ABSOLUTE amount. Compared in whole cents at evaluation time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AmountSpec {
    Eq(f64),
    Gt(f64),
    Ge(f64),
    Lt(f64),
    Le(f64),
    /// Inclusive on both ends; at least one end is set.
    Range(Option<f64>, Option<f64>),
}

/// A date condition resolved to inclusive `yyyymmdd` bounds (`None` = open). `text` is the value as
/// written (lowercased), used by `canonical()`.
#[derive(Debug, Clone, PartialEq)]
pub struct DateSpec {
    pub lo: Option<u32>,
    pub hi: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Payee,
    Account,
    Category,
    LmCategory,
    Notes,
    Id,
    Date,
    Amount,
    Is,
    Sort,
}

impl Field {
    /// Every accepted spelling (lowercase), first the canonical short form.
    pub const NAMES: &'static [(&'static str, Field)] = &[
        ("p", Field::Payee),
        ("payee", Field::Payee),
        ("a", Field::Account),
        ("acct", Field::Account),
        ("account", Field::Account),
        ("c", Field::Category),
        ("cat", Field::Category),
        ("category", Field::Category),
        ("lm", Field::LmCategory),
        ("n", Field::Notes),
        ("note", Field::Notes),
        ("notes", Field::Notes),
        ("id", Field::Id),
        ("d", Field::Date),
        ("date", Field::Date),
        ("amount", Field::Amount),
        ("amt", Field::Amount),
        ("is", Field::Is),
        ("sort", Field::Sort),
    ];

    pub fn from_name(s: &str) -> Option<Field> {
        Field::NAMES.iter().find(|(n, _)| *n == s).map(|(_, f)| *f)
    }

    /// Canonical spelling used by `canonical()`.
    pub fn short(self) -> &'static str {
        match self {
            Field::Payee => "p",
            Field::Account => "a",
            Field::Category => "c",
            Field::LmCategory => "lm",
            Field::Notes => "n",
            Field::Id => "id",
            Field::Date => "d",
            Field::Amount => "amount",
            Field::Is => "is",
            Field::Sort => "sort",
        }
    }

    /// Descriptive name used in JSON.
    pub fn long(self) -> &'static str {
        match self {
            Field::Payee => "payee",
            Field::Account => "account",
            Field::Category => "category",
            Field::LmCategory => "lm_category",
            Field::Notes => "notes",
            Field::Id => "id",
            Field::Date => "date",
            Field::Amount => "amount",
            Field::Is => "is",
            Field::Sort => "sort",
        }
    }
}

/// Quote a value for canonical output when it would not survive bare.
pub(crate) fn quote(s: &str, bare_text: bool) -> String {
    let plain_char = |c: char| !c.is_whitespace() && !c.is_control() && c != '"' && c != ',' && c != '\\' && !(bare_text && c == ':');
    let first_ok = !bare_text || s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
    if !s.is_empty() && first_ok && s.chars().all(plain_char) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn num(x: f64) -> String {
    format!("{}", x)
}

fn amount_text(a: &AmountSpec, in_list: bool) -> String {
    // In a list every item must start with a non-digit, or a comma followed by three digits would be
    // read as a thousands separator.
    match *a {
        AmountSpec::Eq(v) if in_list => format!("={}", num(v)),
        AmountSpec::Eq(v) => num(v),
        AmountSpec::Gt(v) => format!(">{}", num(v)),
        AmountSpec::Ge(v) => format!(">={}", num(v)),
        AmountSpec::Lt(v) => format!("<{}", num(v)),
        AmountSpec::Le(v) => format!("<={}", num(v)),
        AmountSpec::Range(lo, hi) => {
            let lo = lo.map(|v| format!("{}{}", if in_list { "$" } else { "" }, num(v))).unwrap_or_default();
            let hi = hi.map(num).unwrap_or_default();
            format!("{}..{}", lo, hi)
        }
    }
}

impl Term {
    /// `(field, value)` where `field` is `None` for free text. The value is canonical (quoted if needed).
    pub fn parts(&self) -> (Option<Field>, String) {
        let list = |v: &[String]| v.iter().map(|s| quote(s, false)).collect::<Vec<_>>().join(",");
        match &self.kind {
            TermKind::Text(t) => (None, quote(t, true)),
            TermKind::Payee(v) => (Some(Field::Payee), list(v)),
            TermKind::Account(v) => (Some(Field::Account), list(v)),
            TermKind::LmCategory(v) => (Some(Field::LmCategory), list(v)),
            TermKind::Notes(v) => (Some(Field::Notes), list(v)),
            TermKind::Id(v) => (Some(Field::Id), v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")),
            TermKind::Category(v) => {
                let items: Vec<String> = v
                    .iter()
                    .map(|c| match c {
                        CatSpec::None => "none".to_string(),
                        CatSpec::Path(segs) => quote(&segs.join("/"), false),
                    })
                    .collect();
                (Some(Field::Category), items.join(","))
            }
            TermKind::Date(v) => (Some(Field::Date), v.iter().map(|d| d.text.clone()).collect::<Vec<_>>().join(",")),
            TermKind::Amount(v) => {
                let multi = v.len() > 1;
                (Some(Field::Amount), v.iter().map(|a| amount_text(a, multi)).collect::<Vec<_>>().join(","))
            }
            TermKind::Is(v) => (Some(Field::Is), v.iter().map(|k| k.name()).collect::<Vec<_>>().join(",")),
        }
    }

    /// Canonical text of this term without the negation prefix: `p:amazon`, `"harris teeter"`.
    pub fn text(&self) -> String {
        match self.parts() {
            (None, v) => v,
            (Some(f), v) => format!("{}:{}", f.short(), v),
        }
    }

    pub fn canonical(&self) -> String {
        format!("{}{}", if self.neg { "-" } else { "" }, self.text())
    }
}

impl Search {
    /// Normalised text that re-parses (with the same `today`) to an equal `Search`.
    pub fn canonical(&self) -> String {
        let mut parts: Vec<String> = self.terms.iter().map(Term::canonical).collect();
        if let Some((k, d)) = self.sort {
            parts.push(format!("sort:{}", sort_text(k, d)));
        }
        parts.join(" ")
    }

    /// The sort directive as the `sort:` value (`-amount`), if any.
    pub fn sort_text(&self) -> Option<String> {
        self.sort.map(|(k, d)| sort_text(k, d))
    }

    pub fn compile(&self, cache: &crate::cache::Cache) -> Compiled {
        eval::compile(self, cache)
    }
}

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod parse_tests;
#[cfg(test)]
mod eval_tests;
#[cfg(test)]
mod complete_tests;
