//! String similarity for the fuzzy tier, and the token rule for the amount-band tier. Pure.

use std::collections::{HashMap, HashSet};

pub type Gram = [char; 3];

/// Sorted, unique padded trigrams: `"amazon"` -> `"  a"`, `" am"`, `"ama"`, ... `"on "`.
pub fn trigrams(s: &str) -> Vec<Gram> {
    let mut cs: Vec<char> = Vec::with_capacity(s.len() + 3);
    cs.extend([' ', ' ']);
    cs.extend(s.chars());
    cs.push(' ');
    let mut out: Vec<Gram> = cs.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Jaccard of two sorted, unique gram lists, and the intersection size.
pub fn jaccard(a: &[Gram], b: &[Gram]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (mut i, mut j, mut inter) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

fn jaro(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let range = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut am = vec![false; a.len()];
    let mut bm = vec![false; b.len()];
    let mut m = 0usize;
    for i in 0..a.len() {
        let lo = i.saturating_sub(range);
        let hi = (i + range + 1).min(b.len());
        for j in lo..hi {
            if !bm[j] && a[i] == b[j] {
                am[i] = true;
                bm[j] = true;
                m += 1;
                break;
            }
        }
    }
    if m == 0 {
        return 0.0;
    }
    let mut t = 0usize;
    let mut k = 0usize;
    for i in 0..a.len() {
        if am[i] {
            while !bm[k] {
                k += 1;
            }
            if a[i] != b[k] {
                t += 1;
            }
            k += 1;
        }
    }
    let m = m as f64;
    let t = t as f64 / 2.0;
    (m / a.len() as f64 + m / b.len() as f64 + (m - t) / m) / 3.0
}

/// Jaro-Winkler with the usual prefix scale 0.1 and prefix length up to 4.
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let j = jaro(&ac, &bc);
    let prefix = ac.iter().zip(bc.iter()).take(4).take_while(|(x, y)| x == y).count() as f64;
    j + prefix * 0.1 * (1.0 - j)
}

/// The fuzzy tier's score: half trigram Jaccard, half Jaro-Winkler.
pub fn combined(a: &str, b: &str, ga: &[Gram], gb: &[Gram]) -> f64 {
    0.5 * jaccard(ga, gb) + 0.5 * jaro_winkler(a, b)
}

/// Payee words that say nothing about the merchant. Shared stop words never link two payees.
const STOP: &[&str] = &[
    "the", "and", "of", "inc", "llc", "co", "com", "net", "www", "store", "shop", "card", "purchase", "payment", "pmt",
    "online", "debit", "pos", "ach", "us", "usa",
];

/// Non-stop words of a normalized payee, deduplicated, in order.
pub fn tokens(norm: &str) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for t in norm.split(' ') {
        if t.is_empty() || STOP.contains(&t) || t.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        if seen.insert(t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Trigram postings over a set of normalized payees, for "which payees look like this one".
pub struct TrigramIndex {
    grams: Vec<Vec<Gram>>,
    postings: HashMap<Gram, Vec<u32>>,
}

impl TrigramIndex {
    pub fn build(norms: &[String]) -> TrigramIndex {
        let grams: Vec<Vec<Gram>> = norms.iter().map(|s| trigrams(s)).collect();
        let mut postings: HashMap<Gram, Vec<u32>> = HashMap::new();
        for (i, gs) in grams.iter().enumerate() {
            for g in gs {
                postings.entry(*g).or_default().push(i as u32);
            }
        }
        TrigramIndex { grams, postings }
    }

    /// Payees other than `of` whose combined score with it is at least `min`, with their scores.
    pub fn similar(&self, norms: &[String], of: u32, min: f64) -> Vec<(u32, f64)> {
        let q = &self.grams[of as usize];
        let mut shared: HashMap<u32, u32> = HashMap::new();
        for g in q {
            if let Some(list) = self.postings.get(g) {
                for &c in list {
                    *shared.entry(c).or_insert(0) += 1;
                }
            }
        }
        let mut out = Vec::new();
        for (c, _) in shared {
            if c == of {
                continue;
            }
            let score = combined(&norms[of as usize], &norms[c as usize], q, &self.grams[c as usize]);
            if score >= min {
                out.push((c, score));
            }
        }
        out.sort_unstable_by_key(|(c, _)| *c);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaro_winkler_known_values() {
        assert!((jaro_winkler("martha", "marhta") - 0.9611).abs() < 1e-3);
        assert!((jaro_winkler("dwayne", "duane") - 0.84).abs() < 1e-2);
        assert_eq!(jaro_winkler("abc", "abc"), 1.0);
        assert_eq!(jaro_winkler("abc", "xyz"), 0.0);
    }

    #[test]
    fn typo_scores_high_and_unrelated_low() {
        let (a, b) = ("starbucks coffee", "starbucks coffe");
        let s = combined(a, b, &trigrams(a), &trigrams(b));
        assert!(s > 0.9, "{s}");
        let (c, d) = ("netflix", "shell oil");
        assert!(combined(c, d, &trigrams(c), &trigrams(d)) < 0.5);
    }

    #[test]
    fn tokens_skip_stop_words_and_numbers() {
        assert_eq!(tokens("the home depot 12 store"), vec!["home", "depot"]);
        assert_eq!(tokens("amazon amazon"), vec!["amazon"]);
    }

    #[test]
    fn index_finds_typos_only() {
        let norms: Vec<String> = ["starbucks coffe", "starbucks coffee", "shell oil", "netflix"].iter().map(|s| s.to_string()).collect();
        let idx = TrigramIndex::build(&norms);
        let hits = idx.similar(&norms, 1, 0.75);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 0);
    }
}
