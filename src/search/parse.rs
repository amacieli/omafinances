//! The slash-search parser. Total: any input gives `Ok` or a positioned `ParseError`, never a panic.
//!
//! Pipeline: `split` finds the whitespace-separated tokens (quote- and escape-aware), `decode` turns a
//! token into characters tagged "quoted or not", then the token is read as `[-]field:value[,value...]`
//! or as a bare word. Error positions are byte offsets into the original input.

use super::*;
use chrono::{Datelike, Days, Months, NaiveDate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset of the offending text.
    pub pos: usize,
    /// Byte length of the offending text (at least 1 when the input is not empty).
    pub len: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.pos)
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "pos": self.pos, "len": self.len, "message": self.message })
    }
}

/// Date presets, in the order the completer lists them.
pub const PRESETS: &[&str] = &[
    "today", "yesterday", "this-week", "last-week", "this-month", "last-month", "this-quarter", "last-quarter", "ytd", "this-year", "last-year",
];

/// Largest `N` in `Nd`/`Nw`/`Nm`/`Ny`.
const MAX_REL: u64 = 100_000;

// ---- tokenizer --------------------------------------------------------------------------------

/// Byte span of one token. Whitespace inside double quotes does not split; `\"` and `\\` are escapes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RawTok {
    pub start: usize,
    pub end: usize,
}

/// Tokens plus the byte position of an unterminated opening quote, if any.
pub(crate) fn split(input: &str) -> (Vec<RawTok>, Option<usize>) {
    let mut toks = Vec::new();
    let mut start: Option<usize> = None;
    let mut inq = false;
    let mut open = 0usize;
    let mut it = input.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c == '\\' && matches!(it.peek(), Some(&(_, '"')) | Some(&(_, '\\'))) {
            start.get_or_insert(i);
            it.next();
        } else if c == '"' {
            start.get_or_insert(i);
            if !inq {
                open = i;
            }
            inq = !inq;
        } else if !inq && c.is_whitespace() {
            if let Some(s) = start.take() {
                toks.push(RawTok { start: s, end: i });
            }
        } else {
            start.get_or_insert(i);
        }
    }
    if let Some(s) = start {
        toks.push(RawTok { start: s, end: input.len() });
    }
    (toks, if inq { Some(open) } else { None })
}

#[derive(Debug, Clone, Copy)]
struct Ch {
    c: char,
    /// Inside quotes or escaped: never syntax.
    quoted: bool,
    start: usize,
    end: usize,
}

fn decode(input: &str, tok: RawTok) -> Vec<Ch> {
    let mut out = Vec::new();
    let mut inq = false;
    let mut it = input[tok.start..tok.end].char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let a = tok.start + i;
        if c == '\\' {
            if let Some(&(_, n @ ('"' | '\\'))) = it.peek() {
                it.next();
                out.push(Ch { c: n, quoted: true, start: a, end: a + 1 + n.len_utf8() });
                continue;
            }
        }
        if c == '"' {
            inq = !inq;
            continue;
        }
        out.push(Ch { c, quoted: inq, start: a, end: a + c.len_utf8() });
    }
    out
}

fn text_of(chars: &[Ch]) -> String {
    chars.iter().map(|c| c.c).collect()
}

// ---- errors -----------------------------------------------------------------------------------

fn err(input: &str, pos: usize, len: usize, message: String) -> ParseError {
    let mut pos = pos.min(input.len());
    if pos >= input.len() && !input.is_empty() {
        pos = input.len() - 1;
        while !input.is_char_boundary(pos) {
            pos -= 1;
        }
    }
    let len = len.max(1).min(input.len().saturating_sub(pos).max(1));
    ParseError { pos, len, message }
}

/// Shorten a user-typed value echoed in a message.
fn shown(s: &str) -> String {
    let t: String = s.chars().take(40).collect();
    if t.len() < s.len() {
        format!("{}…", t)
    } else {
        t
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur.push(sub.min(prev[j] + 1).min(cur[j - 1] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Best candidate for a misspelt word: a candidate it is a prefix of, else the nearest within a small
/// edit distance. Candidates are `(spelling, what to suggest)`.
fn suggest<'a>(typed: &str, cands: &[(&'a str, &'a str)]) -> Option<&'a str> {
    if typed.is_empty() {
        return None;
    }
    if let Some((_, s)) = cands.iter().find(|(n, _)| n.starts_with(typed)) {
        return Some(s);
    }
    let limit = if typed.chars().count() <= 3 { 1 } else { 2 };
    cands
        .iter()
        .map(|(n, s)| (levenshtein(typed, n), *s))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, s)| s)
}

fn did_you_mean(what: &str, typed: &str, cands: &[(&str, &str)], list: &str) -> String {
    match suggest(typed, cands) {
        Some(s) => format!("unknown {} \"{}\"; did you mean \"{}\"?", what, shown(typed), s),
        None => format!("unknown {} \"{}\"; {}", what, shown(typed), list),
    }
}

// ---- entry point ------------------------------------------------------------------------------

pub fn parse(input: &str, today: NaiveDate) -> Result<Search, ParseError> {
    let (toks, unterminated) = split(input);
    if let Some(p) = unterminated {
        return Err(err(input, p, input.len() - p, "unterminated quote; add a closing \"".to_string()));
    }
    let mut terms = Vec::new();
    let mut sort = None;
    for tok in toks {
        let chars = decode(input, tok);
        if let Some(t) = parse_token(input, tok, &chars, today, &mut sort)? {
            terms.push(t);
        }
    }
    Ok(Search { terms, sort })
}

struct Item {
    text: String,
    start: usize,
    end: usize,
}

fn parse_token(
    input: &str,
    tok: RawTok,
    chars: &[Ch],
    today: NaiveDate,
    sort: &mut Option<(SortKey, Dir)>,
) -> Result<Option<Term>, ParseError> {
    if chars.is_empty() {
        return Err(err(input, tok.start, tok.end - tok.start, "empty term".to_string()));
    }
    let mut neg = false;
    let mut body = chars;
    if chars[0].c == '-' && !chars[0].quoted {
        neg = true;
        body = &chars[1..];
        if body.is_empty() {
            return Err(err(input, tok.start, 1, "nothing to negate after \"-\"".to_string()));
        }
    }

    let colon = body.iter().position(|c| c.c == ':' && !c.quoted);
    let Some(ci) = colon.filter(|&ci| body[..ci].iter().all(|c| !c.quoted)) else {
        return Ok(Some(Term { neg, kind: bare_term(input, body)? }));
    };

    let name_span = (body[0].start, body[ci].start);
    if ci == 0 {
        return Err(err(input, body[0].start, 1, "missing field name before \":\"".to_string()));
    }
    let name = text_of(&body[..ci]).to_lowercase();
    let Some(field) = Field::from_name(&name) else {
        let cands: Vec<(&str, &str)> = Field::NAMES.iter().map(|(n, f)| (*n, f.short())).collect();
        let msg = did_you_mean("field", &name, &cands, "fields are p, a, c, lm, n, id, d, amount, is, sort");
        return Err(err(input, name_span.0, name_span.1 - name_span.0, msg));
    };

    let value = &body[ci + 1..];
    let value_start = body[ci].end;
    if value.is_empty() {
        // `p:""` also lands here: the quotes decode to nothing.
        return Err(err(input, body[ci].start, body[ci].end - body[ci].start, format!("missing value after \"{}:\"", name)));
    }
    let items = split_items(input, tok, value, value_start, field == Field::Amount)?;

    if field == Field::Sort {
        if neg {
            return Err(err(input, tok.start, 1, "sort cannot be negated".to_string()));
        }
        if items.len() != 1 {
            return Err(err(input, items[1].start, items[1].end - items[1].start, "sort takes a single value".to_string()));
        }
        if sort.is_some() {
            return Err(err(input, tok.start, tok.end - tok.start, "sort given more than once".to_string()));
        }
        let it = &items[0];
        let v = it.text.to_lowercase();
        let (dir, key) = match v.strip_prefix('-') {
            Some(k) => (Dir::Desc, k),
            None => (Dir::Asc, v.as_str()),
        };
        let Some(k) = SortKey::from_name(key) else {
            let cands: Vec<(&str, &str)> = SortKey::ALL.iter().map(|k| (k.name(), k.name())).collect();
            let msg = did_you_mean("sort key", key, &cands, "keys are date, amount, payee, account, category");
            return Err(err(input, it.start, it.end - it.start, msg));
        };
        *sort = Some((k, dir));
        return Ok(None);
    }

    let kind = match field {
        Field::Payee => TermKind::Payee(strings(input, &items)?),
        Field::Account => TermKind::Account(strings(input, &items)?),
        Field::LmCategory => TermKind::LmCategory(strings(input, &items)?),
        Field::Notes => TermKind::Notes(strings(input, &items)?),
        Field::Id => {
            let mut v = Vec::new();
            for it in &items {
                match it.text.parse::<i64>() {
                    Ok(n) if it.text.bytes().all(|b| b.is_ascii_digit()) => v.push(n),
                    _ => return Err(err(input, it.start, it.end - it.start, format!("bad id \"{}\"; ids are whole numbers", shown(&it.text)))),
                }
            }
            TermKind::Id(v)
        }
        Field::Category => {
            let mut v = Vec::new();
            for it in &items {
                v.push(parse_cat(&it.text).map_err(|m| err(input, it.start, it.end - it.start, m))?);
            }
            TermKind::Category(v)
        }
        Field::Date => {
            let mut v = Vec::new();
            for it in &items {
                v.push(parse_date_item(&it.text, today).map_err(|m| err(input, it.start, it.end - it.start, m))?);
            }
            TermKind::Date(v)
        }
        Field::Amount => {
            let mut v = Vec::new();
            for it in &items {
                v.push(parse_amount_item(&it.text).map_err(|m| err(input, it.start, it.end - it.start, m))?);
            }
            TermKind::Amount(v)
        }
        Field::Is => {
            let mut v = Vec::new();
            for it in &items {
                let t = it.text.to_lowercase();
                match IsKind::from_name(&t) {
                    Some(k) => v.push(k),
                    None => {
                        let cands: Vec<(&str, &str)> = IsKind::ALL.iter().map(|k| (k.name(), k.name())).collect();
                        let msg = did_you_mean("is: value", &t, &cands, "values are recurring, pending, transfer, income, expense, uncat, categorized");
                        return Err(err(input, it.start, it.end - it.start, msg));
                    }
                }
            }
            TermKind::Is(v)
        }
        Field::Sort => unreachable!("handled above"),
    };
    Ok(Some(Term { neg, kind }))
}

fn strings(input: &str, items: &[Item]) -> Result<Vec<String>, ParseError> {
    items
        .iter()
        .map(|it| {
            if it.text.is_empty() {
                Err(err(input, it.start, it.end - it.start, "empty value".to_string()))
            } else {
                Ok(it.text.to_lowercase())
            }
        })
        .collect()
}

/// True when the number being read at the end of `cur` already has a decimal point (a lone `.`,
/// not the `..` of a range), so a following comma cannot be a thousands separator.
fn in_decimal(cur: &[Ch]) -> bool {
    let text: String = cur.iter().map(|c| c.c).collect();
    text.rsplit("..").next().is_some_and(|last| last.contains('.'))
}

/// Split a value on unquoted commas. For amounts a comma between a digit and exactly three digits
/// (`1,200`) is a thousands separator, not an OR.
fn split_items(input: &str, tok: RawTok, value: &[Ch], value_start: usize, thousands: bool) -> Result<Vec<Item>, ParseError> {
    let mut items = Vec::new();
    let mut cur: Vec<Ch> = Vec::new();
    let mut item_start = value_start;
    for (j, ch) in value.iter().enumerate() {
        let is_sep = ch.c == ',' && !ch.quoted && {
            let grouping = thousands
                && j > 0
                && value[j - 1].c.is_ascii_digit()
                && !in_decimal(&cur)
                && value.get(j + 1..j + 4).is_some_and(|w| w.len() == 3 && w.iter().all(|c| c.c.is_ascii_digit()))
                && !value.get(j + 4).is_some_and(|c| c.c.is_ascii_digit());
            !grouping
        };
        if is_sep {
            items.push(Item { text: text_of(&cur), start: item_start, end: ch.start });
            cur.clear();
            item_start = ch.end;
        } else {
            cur.push(*ch);
        }
    }
    items.push(Item { text: text_of(&cur), start: item_start, end: tok.end });
    for it in &items {
        if it.text.is_empty() {
            return Err(err(input, it.start, it.end.saturating_sub(it.start), "empty value in list".to_string()));
        }
    }
    Ok(items)
}

fn bare_term(input: &str, body: &[Ch]) -> Result<TermKind, ParseError> {
    let text = text_of(body);
    if body.iter().any(|c| c.quoted) {
        if text.is_empty() {
            return Err(err(input, body[0].start, 1, "empty phrase".to_string()));
        }
        return Ok(TermKind::Text(text.to_lowercase()));
    }
    let amountish = text.chars().all(|c| c.is_ascii_digit() || "$,.<>=".contains(c));
    if amountish {
        let committed = text.starts_with(['<', '>', '=', '$']) || text.contains("..");
        match parse_amount_item(&text) {
            Ok(a) => return Ok(TermKind::Amount(vec![a])),
            Err(m) if committed => return Err(err(input, body[0].start, body[body.len() - 1].end - body[0].start, m)),
            Err(_) => {}
        }
    }
    Ok(TermKind::Text(text.to_lowercase()))
}

// ---- categories -------------------------------------------------------------------------------

fn parse_cat(text: &str) -> Result<CatSpec, String> {
    let l = text.to_lowercase();
    if l.trim() == "none" {
        return Ok(CatSpec::None);
    }
    let segs: Vec<String> = l.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Err("empty category path; use c:home, c:home/maintenance or c:none".to_string());
    }
    Ok(CatSpec::Path(segs))
}

// ---- amounts ----------------------------------------------------------------------------------

fn bad_amount(t: &str) -> String {
    format!("bad amount \"{}\"; use 50, >50, >=50, <20, 10..50, ..50 or $1,200", shown(t))
}

fn parse_num(s: &str) -> Option<f64> {
    let s = s.strip_prefix('$').unwrap_or(s);
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    let int_clean: String = if int.contains(',') {
        let groups: Vec<&str> = int.split(',').collect();
        let ok = (1..=3).contains(&groups[0].len()) && groups[1..].iter().all(|g| g.len() == 3) && groups.iter().all(|g| digits(g));
        if !ok {
            return None;
        }
        groups.concat()
    } else {
        if !digits(int) {
            return None;
        }
        int.to_string()
    };
    if let Some(f) = frac {
        if f.is_empty() || !digits(f) {
            return None;
        }
    }
    if int_clean.is_empty() && frac.is_none() {
        return None;
    }
    if int_clean.len() + frac.map_or(0, str::len) > 15 {
        return None;
    }
    let joined = format!("{}{}{}", if int_clean.is_empty() { "0" } else { &int_clean }, if frac.is_some() { "." } else { "" }, frac.unwrap_or(""));
    joined.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn parse_amount_item(t: &str) -> Result<AmountSpec, String> {
    let t = t.trim();
    if let Some((a, b)) = t.split_once("..") {
        let side = |s: &str| -> Result<Option<f64>, String> {
            if s.is_empty() {
                Ok(None)
            } else {
                parse_num(s).map(Some).ok_or_else(|| bad_amount(t))
            }
        };
        let (lo, hi) = (side(a)?, side(b)?);
        if lo.is_none() && hi.is_none() {
            return Err(bad_amount(t));
        }
        if let (Some(l), Some(h)) = (lo, hi) {
            if l > h {
                return Err(format!("range \"{}\" starts above its end", shown(t)));
            }
        }
        return Ok(AmountSpec::Range(lo, hi));
    }
    let (op, rest) = strip_op(t);
    let v = parse_num(rest).ok_or_else(|| bad_amount(t))?;
    Ok(match op {
        ">=" => AmountSpec::Ge(v),
        "<=" => AmountSpec::Le(v),
        ">" => AmountSpec::Gt(v),
        "<" => AmountSpec::Lt(v),
        _ => AmountSpec::Eq(v),
    })
}

fn strip_op(t: &str) -> (&'static str, &str) {
    for op in [">=", "<=", ">", "<", "="] {
        if let Some(r) = t.strip_prefix(op) {
            return (if op == "=" { "" } else { op }, r);
        }
    }
    ("", t)
}

// ---- dates ------------------------------------------------------------------------------------

fn key(d: NaiveDate) -> u32 {
    (d.year().max(0) as u32) * 10000 + d.month() * 100 + d.day()
}

fn month_start(y: i32, m: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, 1)
}

fn month_end(y: i32, m: u32) -> Option<NaiveDate> {
    month_start(y, m)?.checked_add_months(Months::new(1))?.pred_opt()
}

/// Inclusive `(start, end)` of a preset relative to `today`.
pub fn preset_range(name: &str, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let (y, m) = (today.year(), today.month());
    Some(match name {
        "today" => (today, today),
        "yesterday" => {
            let d = today.pred_opt()?;
            (d, d)
        }
        "this-week" | "last-week" => {
            let mon = today.checked_sub_days(Days::new(today.weekday().num_days_from_monday() as u64))?;
            let mon = if name == "last-week" { mon.checked_sub_days(Days::new(7))? } else { mon };
            (mon, mon.checked_add_days(Days::new(6))?)
        }
        "this-month" => (month_start(y, m)?, month_end(y, m)?),
        "last-month" => {
            let first = month_start(y, m)?.checked_sub_months(Months::new(1))?;
            (first, month_end(first.year(), first.month())?)
        }
        "this-quarter" | "last-quarter" => {
            let qs = month_start(y, (m - 1) / 3 * 3 + 1)?;
            let qs = if name == "last-quarter" { qs.checked_sub_months(Months::new(3))? } else { qs };
            (qs, qs.checked_add_months(Months::new(3))?.pred_opt()?)
        }
        "ytd" => (NaiveDate::from_ymd_opt(y, 1, 1)?, today),
        "this-year" => (NaiveDate::from_ymd_opt(y, 1, 1)?, NaiveDate::from_ymd_opt(y, 12, 31)?),
        "last-year" => (NaiveDate::from_ymd_opt(y - 1, 1, 1)?, NaiveDate::from_ymd_opt(y - 1, 12, 31)?),
        _ => return None,
    })
}

/// `2026`, `2026-10`, `2026-10-05` as an inclusive period. `None` when it is not date-shaped, `Some(Err)`
/// when it is date-shaped but not a real date.
fn period(s: &str) -> Option<Result<(NaiveDate, NaiveDate), String>> {
    let parts: Vec<&str> = s.split('-').collect();
    let lens_ok = match parts.len() {
        1 => parts[0].len() == 4,
        2 => parts[0].len() == 4 && parts[1].len() == 2,
        3 => parts[0].len() == 4 && parts[1].len() == 2 && parts[2].len() == 2,
        _ => false,
    };
    if !lens_ok || !parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let n: Vec<u32> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
    let bad = || Err(format!("\"{}\" is not a real date", shown(s)));
    let y = n[0] as i32;
    if y < 1 {
        return Some(bad());
    }
    Some(match n.len() {
        1 => match (NaiveDate::from_ymd_opt(y, 1, 1), NaiveDate::from_ymd_opt(y, 12, 31)) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => bad(),
        },
        2 => match (month_start(y, n[1]), month_end(y, n[1])) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => bad(),
        },
        _ => match NaiveDate::from_ymd_opt(y, n[1], n[2]) {
            Some(d) => Ok((d, d)),
            None => bad(),
        },
    })
}

fn relative(s: &str) -> Option<(u64, char)> {
    let unit = s.chars().last()?;
    if !matches!(unit, 'd' | 'w' | 'm' | 'y') {
        return None;
    }
    let digits = &s[..s.len() - 1];
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, unit))
}

fn back(today: NaiveDate, n: u64, unit: char) -> Option<NaiveDate> {
    if n > MAX_REL {
        return None;
    }
    match unit {
        'd' => today.checked_sub_days(Days::new(n)),
        'w' => today.checked_sub_days(Days::new(n * 7)),
        'm' => today.checked_sub_months(Months::new(n as u32)),
        _ => today.checked_sub_months(Months::new((n as u32).checked_mul(12)?)),
    }
}

fn parse_date_item(t: &str, today: NaiveDate) -> Result<DateSpec, String> {
    let text = t.to_lowercase();
    let t = text.as_str();
    let spec = |lo: Option<NaiveDate>, hi: Option<NaiveDate>| DateSpec { lo: lo.map(key), hi: hi.map(key), text: text.clone() };
    let range_err = || "date out of range".to_string();

    if PRESETS.contains(&t) {
        let (a, b) = preset_range(t, today).ok_or_else(range_err)?;
        return Ok(spec(Some(a), Some(b)));
    }
    if let Some((a, b)) = t.split_once("..") {
        let end = |s: &str, upper: bool| -> Result<Option<NaiveDate>, String> {
            if s.is_empty() {
                return Ok(None);
            }
            match period(s) {
                Some(Ok((lo, hi))) => Ok(Some(if upper { hi } else { lo })),
                Some(Err(m)) => Err(m),
                None => Err(bad_date(t)),
            }
        };
        let (lo, hi) = (end(a, false)?, end(b, true)?);
        if lo.is_none() && hi.is_none() {
            return Err(bad_date(t));
        }
        if let (Some(l), Some(h)) = (lo, hi) {
            if l > h {
                return Err(format!("range \"{}\" starts after its end", shown(t)));
            }
        }
        return Ok(spec(lo, hi));
    }
    let (op, rest) = strip_op(t);
    if let Some((n, unit)) = relative(rest) {
        let anchor = back(today, n, unit).ok_or_else(|| "that is too far back".to_string())?;
        let next = anchor.succ_opt().ok_or_else(range_err)?;
        let prev = anchor.pred_opt().ok_or_else(range_err)?;
        return match op {
            ">" => Ok(spec(Some(next), None)),
            ">=" => Ok(spec(Some(anchor), None)),
            "<" => Ok(spec(None, Some(prev))),
            "<=" => Ok(spec(None, Some(anchor))),
            _ if t.starts_with('=') => Err(bad_date(t)),
            _ => Ok(spec(Some(next), Some(today))),
        };
    }
    match period(rest) {
        Some(Ok((lo, hi))) => {
            let one = |d: Option<NaiveDate>| d.ok_or_else(range_err);
            match op {
                ">" => Ok(spec(Some(one(hi.succ_opt())?), None)),
                ">=" => Ok(spec(Some(lo), None)),
                "<" => Ok(spec(None, Some(one(lo.pred_opt())?))),
                "<=" => Ok(spec(None, Some(hi))),
                _ => Ok(spec(Some(lo), Some(hi))),
            }
        }
        Some(Err(m)) => Err(m),
        None => {
            if !t.is_empty() && t.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
                let cands: Vec<(&str, &str)> = PRESETS.iter().map(|p| (*p, *p)).collect();
                return Err(did_you_mean("date preset", t, &cands, "presets are today, yesterday, this-week, this-month, last-month, ytd, this-year, ..."));
            }
            Err(bad_date(t))
        }
    }
}

fn bad_date(t: &str) -> String {
    format!("bad date \"{}\"; use 2026, 2026-10, 2026-10-05, a range a..b, >2026-01-01, a preset like this-month, or 30d, 3m, 1y", shown(t))
}
