//! Payee normalizer: the key that rows are grouped by. `normalize(raw, &aliases)` is pure.
//!
//! Order: `payee_alias` rows first (exact or regex, first match wins, the canonical text is the result), then
//! prefix stripping (`pos `, `sq *`, `tst*`, `paypal *`, `pp*`, `debit card purchase`, and Amazon's marketplace
//! prefixes mapped to `amazon`), then tokens: anything containing a digit (store and terminal numbers, `#12`,
//! card tails `xxxx1234`, dates) is dropped, and the first number that follows a kept word ends the name, so
//! the store's city and state after it go too. A trailing upper-case US state code is dropped (`... PORTLAND OR`).

use regex::Regex;

/// One `payee_alias` row. Regex rows are matched case-insensitively against the collapsed lowercase payee.
#[derive(Debug, Clone)]
pub struct AliasRow {
    pub pattern: String,
    pub canonical: String,
    pub is_regex: bool,
}

/// Compiled aliases. Build once per request; invalid regexes are skipped.
#[derive(Debug, Default)]
pub struct Aliases {
    exact: Vec<(String, String)>,
    regex: Vec<(Regex, String)>,
}

impl Aliases {
    pub fn none() -> Aliases {
        Aliases::default()
    }

    pub fn compile(rows: &[AliasRow]) -> Aliases {
        let mut a = Aliases::default();
        for r in rows {
            let canon = collapse(&r.canonical.to_lowercase());
            if canon.is_empty() {
                continue;
            }
            if r.is_regex {
                if let Ok(re) = Regex::new(&format!("(?i){}", r.pattern)) {
                    a.regex.push((re, canon));
                }
            } else {
                let pat = collapse(&r.pattern.to_lowercase());
                if !pat.is_empty() {
                    a.exact.push((pat, canon));
                }
            }
        }
        a
    }

    fn lookup(&self, lc: &str) -> Option<&str> {
        self.exact
            .iter()
            .find(|(p, _)| p == lc)
            .map(|(_, c)| c.as_str())
            .or_else(|| self.regex.iter().find(|(re, _)| re.is_match(lc)).map(|(_, c)| c.as_str()))
    }
}

/// Prefix, replacement. Replacements never start with another prefix, so the loop terminates.
const PREFIXES: &[(&str, &str)] = &[
    ("amzn mktp us*", "amazon "),
    ("amazon mktp us*", "amazon "),
    ("amzn mktp", "amazon "),
    ("amazon.com*", "amazon "),
    ("amazon.com", "amazon "),
    ("debit card purchase", ""),
    ("pos ", ""),
    ("sq *", ""),
    ("sq*", ""),
    ("tst*", ""),
    ("tst *", ""),
    ("paypal *", ""),
    ("paypal*", ""),
    ("pp*", ""),
];

const US_STATES: &[&str] = &[
    "al", "ak", "az", "ar", "ca", "co", "ct", "de", "fl", "ga", "hi", "id", "il", "in", "ia", "ks", "ky", "la", "me",
    "md", "ma", "mi", "mn", "ms", "mo", "mt", "ne", "nv", "nh", "nj", "nm", "ny", "nc", "nd", "oh", "ok", "or", "pa",
    "ri", "sc", "sd", "tn", "tx", "ut", "vt", "va", "wa", "wv", "wi", "wy", "dc",
];

/// Lowercase, trim, and collapse runs of whitespace to one space.
pub fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The normalized payee. Falls back to the collapsed lowercase text when nothing is left (e.g. a payee that is
/// only a number), so the result is empty only for an empty payee.
pub fn normalize(raw: &str, aliases: &Aliases) -> String {
    let lc = collapse(&raw.to_lowercase());
    if let Some(canon) = aliases.lookup(&lc) {
        return canon.to_string();
    }
    let mut s = lc.clone();
    loop {
        let hit = PREFIXES.iter().find(|(p, _)| s.starts_with(p));
        match hit {
            Some((p, rep)) => {
                let rest = s[p.len()..].trim_start().to_string();
                s = format!("{rep}{rest}");
            }
            None => break,
        }
    }

    let mut kept: Vec<&str> = Vec::new();
    for tok in s.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()) {
        if tok.chars().any(|c| c.is_ascii_digit()) {
            if kept.is_empty() {
                continue;
            }
            break;
        }
        if matches!(tok, "www") {
            continue;
        }
        kept.push(tok);
    }
    // Trailing `com`/`net`/`org` from a domain-looking payee (`netflix.com`).
    while kept.len() > 1 && matches!(kept.last(), Some(&"com" | &"net" | &"org")) {
        kept.pop();
    }
    // A trailing upper-case state code in the original text (`PORTLAND OR`).
    if kept.len() > 1 {
        if let Some(last_raw) = raw.split_whitespace().last() {
            let upper2 = last_raw.len() == 2 && last_raw.chars().all(|c| c.is_ascii_uppercase());
            if upper2 && kept.last().is_some_and(|t| US_STATES.contains(t)) {
                kept.pop();
            }
        }
    }
    if kept.is_empty() {
        return lc;
    }
    kept.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(raw: &str) -> String {
        normalize(raw, &Aliases::none())
    }

    #[test]
    fn amazon_variants_collapse_to_one_key() {
        assert_eq!(n("AMZN Mktp US*1A2B3"), "amazon");
        assert_eq!(n("AMAZON MKTP US*9Z8Y7"), "amazon");
        assert_eq!(n("AMAZON.COM*2K3L4"), "amazon");
        assert_eq!(n("Amazon.com"), "amazon");
    }

    #[test]
    fn square_paypal_and_other_prefixes() {
        assert_eq!(n("SQ *COFFEE SHOP #12 PORTLAND OR"), "coffee shop");
        assert_eq!(n("PAYPAL *NETFLIX 4029357733"), "netflix");
        assert_eq!(n("PP*SPOTIFY P1234"), "spotify");
        assert_eq!(n("TST* BLUE BOTTLE 0042 SAN FRANCISCO CA"), "blue bottle");
        assert_eq!(n("POS DEBIT CARD PURCHASE HOME DEPOT 1234 AUSTIN TX"), "home depot");
    }

    #[test]
    fn numbers_card_tails_dates_and_states() {
        assert_eq!(n("SHELL OIL 57442 PORTLAND OR"), "shell oil");
        assert_eq!(n("COSTCO WHOLESALE #1234"), "costco wholesale");
        assert_eq!(n("XXXX1234 STARBUCKS 01/15"), "starbucks");
        assert_eq!(n("WALMART AUSTIN TX"), "walmart austin");
        // a lowercase trailing word is not treated as a state code
        assert_eq!(n("Pizza Hut in"), "pizza hut in");
        assert_eq!(n("Netflix.com"), "netflix");
    }

    #[test]
    fn numeric_only_payee_keeps_its_text() {
        assert_eq!(n("12345"), "12345");
        assert_eq!(n(""), "");
    }

    #[test]
    fn aliases_win_over_everything_and_regex_is_case_insensitive() {
        let a = Aliases::compile(&[
            AliasRow { pattern: "SQ *BLUE BOTTLE".into(), canonical: "Blue Bottle Coffee".into(), is_regex: false },
            AliasRow { pattern: "^uber (eats|trip)".into(), canonical: "Uber".into(), is_regex: true },
            AliasRow { pattern: "([".into(), canonical: "broken".into(), is_regex: true },
        ]);
        assert_eq!(normalize("sq *blue  bottle", &a), "blue bottle coffee");
        assert_eq!(normalize("UBER TRIP 1234 SF", &a), "uber");
        assert_eq!(normalize("Uber Eats", &a), "uber");
        // the broken regex is skipped, not an error
        assert_eq!(normalize("broken thing", &a), "broken thing");
    }
}
