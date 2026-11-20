use super::complete::{complete, Completions, MAX_ITEMS};
use super::fixture::{fixture, today};
use super::*;
use crate::cache::testing::synthetic;
use crate::cache::Cache;
use crate::db::{RawAccount, RawCategory, RawTxn, Snapshot};

/// Payees: Amazon x5, Whole Foods x4, Amazon Prime x3, Samazon Co x2, Bob "B" Cafe x1, Shell x1.
fn freq_cache() -> Cache {
    let mut txns = Vec::new();
    let mut id = 1;
    for (payee, n) in [("Amazon", 5), ("Whole Foods", 4), ("Amazon Prime", 3), ("Samazon Co", 2), ("Shell", 1), ("Bob \"B\" Cafe", 1)] {
        for _ in 0..n {
            txns.push(RawTxn { id, date: "2026-01-02".into(), payee: Some(payee.into()), amount: 1.0, account: Some("Checking".into()), account_id: Some(1), ..Default::default() });
            id += 1;
        }
    }
    let accounts = vec![
        RawAccount { id: 1, name: "Checking".into(), ..Default::default() },
        RawAccount { id: 2, name: "Savings".into(), ..Default::default() },
        RawAccount { id: 3, name: "Credit Card".into(), ..Default::default() },
    ];
    let categories = vec![RawCategory { id: 1, parent_id: None, name: "Home".into(), kind: "expense".into(), sort: 0 }];
    Cache::build(Snapshot { txns, accounts, categories, balances: vec![] })
}

fn comp(c: Option<&Cache>, input: &str, pos: usize) -> Completions {
    complete(c, input, pos, today())
}

fn at_end(c: Option<&Cache>, input: &str) -> Completions {
    comp(c, input, input.len())
}

fn labels(r: &Completions) -> Vec<&str> {
    r.items.iter().map(|i| i.label.as_str()).collect()
}

fn apply(input: &str, r: &Completions, i: usize) -> String {
    format!("{}{}{}", &input[..r.start], r.items[i].insert, &input[r.end..])
}

#[test]
fn field_names_by_name_or_alias_prefix() {
    let r = at_end(None, "");
    assert_eq!((r.start, r.end), (0, 0));
    assert_eq!(labels(&r), vec!["p:", "a:", "c:", "d:", "amount:", "is:", "sort:", "n:", "lm:"]);

    assert_eq!(labels(&at_end(None, "p")), vec!["p:"]);
    assert_eq!(labels(&at_end(None, "a")), vec!["a:", "amount:"]); // exact short name first
    assert_eq!(labels(&at_end(None, "am")), vec!["amount:"]);
    assert_eq!(labels(&at_end(None, "pay")), vec!["p:"]); // alias "payee"
    assert_eq!(labels(&at_end(None, "acc")), vec!["a:"]);
    assert_eq!(labels(&at_end(None, "cat")), vec!["c:"]);
    assert_eq!(labels(&at_end(None, "n")), vec!["n:"]);
    assert_eq!(labels(&at_end(None, "l")), vec!["lm:"]);
    assert_eq!(labels(&at_end(None, "s")), vec!["sort:"]);
    assert!(at_end(None, "zzz").items.is_empty());
    assert!(at_end(None, "\"quoted").items.is_empty());

    let r = at_end(None, "foo p");
    assert_eq!((r.start, r.end), (4, 5));
    assert_eq!(apply("foo p", &r, 0), "foo p:");
    let r = at_end(None, "-am");
    assert_eq!((r.start, r.end), (0, 3));
    assert_eq!(r.items[0].insert, "-amount:");
    assert!(r.items.iter().all(|i| !i.detail.is_empty()));
}

#[test]
fn payees_most_frequent_first_prefix_before_substring() {
    let c = freq_cache();
    let r = at_end(Some(&c), "p:");
    assert_eq!(labels(&r), vec!["Amazon", "Whole Foods", "Amazon Prime", "Samazon Co", "Bob \"B\" Cafe", "Shell"]);
    assert_eq!(r.items[0].detail, "5 txns");
    assert_eq!(r.items[4].detail, "1 txn");
    assert_eq!((r.start, r.end), (0, 2));

    let r = at_end(Some(&c), "p:am");
    assert_eq!(labels(&r), vec!["Amazon", "Amazon Prime", "Samazon Co"]); // prefix hits, then substring hits
    assert_eq!(r.items[0].insert, "p:Amazon");
    let r = at_end(Some(&c), "p:WHO");
    assert_eq!(labels(&r), vec!["Whole Foods"]);
    assert_eq!(r.items[0].insert, "p:\"Whole Foods\""); // quoted: it has a space
    assert_eq!(r.items[0].detail, "4 txns");
    assert!(at_end(Some(&c), "p:zzz").items.is_empty());
    // without a cache there is nothing to offer
    assert!(at_end(None, "p:am").items.is_empty());
}

#[test]
fn what_the_completer_inserts_parses_and_matches() {
    let c = freq_cache();
    for input in ["p:", "p:am", "-p:s", "p:bob"] {
        let r = at_end(Some(&c), input);
        assert!(!r.items.is_empty(), "{}", input);
        for i in 0..r.items.len() {
            let text = apply(input, &r, i);
            let s = parse(&text, today()).unwrap_or_else(|e| panic!("{:?} -> {:?}: {}", input, text, e));
            let comp = s.compile(&c);
            let hits = c.txns.iter().filter(|t| comp.matches(t)).count();
            let positive = if s.terms[0].neg { c.txns.len() - hits } else { hits };
            // the payee we inserted matches at least its own rows
            let own = c.payee_freq.iter().find(|(p, _)| *p == r.items[i].label).unwrap().1;
            assert!(positive >= own, "{}", text);
        }
    }
    let r = at_end(Some(&c), "p:bob");
    assert_eq!(r.items[0].insert, "p:\"Bob \\\"B\\\" Cafe\"");
}

#[test]
fn accounts() {
    let c = freq_cache();
    let r = at_end(Some(&c), "a:");
    assert_eq!(labels(&r), vec!["Checking", "Credit Card", "Savings"]); // busiest first, then by name
    assert_eq!(r.items[0].detail, "16 txns");
    assert_eq!(r.items[1].detail, "0 txns");
    assert_eq!(labels(&at_end(Some(&c), "acct:c")), vec!["Checking", "Credit Card"]);
    assert_eq!(labels(&at_end(Some(&c), "a:vin")), vec!["Savings"]); // substring
    let r = at_end(Some(&c), "a:cred");
    assert_eq!(r.items[0].insert, "a:\"Credit Card\"");
}

#[test]
fn categories_inserted_paths_are_accepted_by_the_matcher() {
    let c = Cache::build(synthetic(600));
    // shallow first, busiest first; `none` is offered too
    let r = at_end(Some(&c), "c:");
    assert!(r.items.iter().take(4).all(|i| !i.label.contains('/')), "{:?}", labels(&r));
    assert!(labels(&r).contains(&"none"));
    // every node can be found by typing its (quoted) path, and the insert matches exactly its subtree
    for node in &c.categories {
        let path = node.path.split(crate::cache::PATH_SEP).collect::<Vec<_>>().join("/");
        let input = format!("c:\"{}\"", path.to_lowercase());
        let r = at_end(Some(&c), &input);
        let i = r.items.iter().position(|it| it.label == path).unwrap_or_else(|| panic!("{} missing from {:?}", path, labels(&r)));
        let text = apply(&input, &r, i);
        assert_eq!(text.starts_with("c:\""), path.contains(' '), "{}", text);
        let s = parse(&text, today()).unwrap_or_else(|e| panic!("{}: {}", text, e));
        let comp = s.compile(&c);
        let hits = c.txns.iter().filter(|t| comp.matches(t)).count();
        assert_eq!(hits, node.count_subtree, "{}", text);
    }
    let r = at_end(Some(&c), "c:\"home/sub 0/");
    assert!(r.items.len() >= 3 && r.items.iter().all(|i| i.label.starts_with("Home/Sub 0")), "{:?}", labels(&r));
    // typing a later segment finds it too
    let r = at_end(Some(&c), "c:leaf");
    assert!(r.items.len() == MAX_ITEMS && r.items.iter().all(|i| i.label.contains("Leaf")));
    // none
    let r = at_end(Some(&c), "c:no");
    assert_eq!(r.items[0].insert, "c:none");
    assert_eq!(r.items[0].detail, format!("{} txns", c.uncategorized));
}

#[test]
fn dates_is_and_sort() {
    let c = fixture();
    let r = at_end(Some(&c), "d:");
    assert_eq!(labels(&r), parse::PRESETS.to_vec());
    let month = r.items.iter().find(|i| i.label == "this-month").unwrap();
    assert_eq!(month.detail, "2026-01-01..2026-01-31");
    assert_eq!(r.items[0].detail, "2026-01-08"); // today is a single day
    let r = at_end(Some(&c), "d:this");
    assert_eq!(labels(&r), vec!["this-week", "this-month", "this-quarter", "this-year"]);
    assert_eq!(apply("d:this", &r, 1), "d:this-month");
    assert_eq!(labels(&at_end(None, "date:y")), vec!["yesterday", "ytd"]);

    let r = at_end(Some(&c), "is:");
    assert_eq!(labels(&r), vec!["recurring", "pending", "transfer", "income", "expense", "uncat", "categorized"]);
    let detail = |name: &str| r.items.iter().find(|i| i.label == name).unwrap().detail.clone();
    assert_eq!(detail("recurring"), "1 txn");
    assert_eq!(detail("transfer"), "2 txns");
    assert_eq!(detail("income"), "2 txns");
    assert_eq!(detail("expense"), "6 txns");
    assert_eq!(detail("categorized"), "7 txns");
    assert_eq!(labels(&at_end(Some(&c), "is:p")), vec!["pending"]);
    assert_eq!(labels(&at_end(Some(&c), "-is:re")), vec!["recurring"]);
    assert_eq!(at_end(Some(&c), "-is:re").items[0].insert, "-is:recurring");

    let r = at_end(None, "sort:");
    assert_eq!(r.items.len(), 10);
    assert_eq!(labels(&r)[..5], ["date", "amount", "payee", "account", "category"]);
    assert_eq!(labels(&at_end(None, "sort:-a")), vec!["-amount", "-account"]);
    assert_eq!(at_end(None, "sort:-a").items[0].insert, "sort:-amount");
    assert_eq!(labels(&at_end(None, "sort:am")), vec!["amount", "-amount"]);
    // nothing for fields without value hints, or unknown fields
    assert!(at_end(Some(&c), "amount:").items.is_empty());
    assert!(at_end(Some(&c), "xyz:").items.is_empty());
}

#[test]
fn cursor_in_the_middle_and_replacement_ranges() {
    let c = freq_cache();
    // end of the first of two tokens
    let input = "p:am c:home";
    let r = comp(Some(&c), input, 4);
    assert_eq!((r.start, r.end), (0, 4));
    assert_eq!(apply(input, &r, 0), "p:Amazon c:home");
    // second token
    let r = at_end(Some(&c), "p:amazon c:ho");
    assert_eq!((r.start, r.end), (9, 13));
    // inside a value: the rest of the item is replaced, the next token kept
    let input = "p:amazon c:home";
    let r = comp(Some(&c), input, 5);
    assert_eq!((r.start, r.end), (0, 8));
    assert_eq!(apply(input, &r, 0), "p:Amazon c:home");
    // in the whitespace between tokens: a fresh token
    let input = "p:x  c:y";
    let r = comp(Some(&c), input, 4);
    assert_eq!((r.start, r.end), (4, 4));
    assert_eq!(labels(&r)[0], "p:");
    assert_eq!(apply(input, &r, 0), "p:x p: c:y");
    // at the very start of the second token it belongs to that token
    let r = comp(Some(&c), "p:x c:y", 4);
    assert_eq!(r.start, 4);
    // OR lists: the head is kept, only the current item changes
    let input = "p:amazon,wh";
    let r = at_end(Some(&c), input);
    assert_eq!((r.start, r.end), (0, 11));
    assert_eq!(apply(input, &r, 0), "p:amazon,\"Whole Foods\"");
    let input = "p:ama,costco";
    let r = comp(Some(&c), input, 5);
    assert_eq!((r.start, r.end), (0, 5));
    assert_eq!(apply(input, &r, 0), "p:Amazon,costco");
    // inside an open quote
    let input = "p:\"whole f";
    let r = at_end(Some(&c), input);
    assert_eq!((r.start, r.end), (0, 10));
    assert_eq!(apply(input, &r, 0), "p:\"Whole Foods\"");
    // a comma inside quotes is not a separator
    let r = comp(Some(&c), "p:\"a,b\" c:", 6);
    assert_eq!((r.start, r.end), (0, 7));
    // negation is kept
    let input = "x -p:am";
    let r = at_end(Some(&c), input);
    assert_eq!((r.start, r.end), (2, 7));
    assert_eq!(apply(input, &r, 0), "x -p:Amazon");
    // cursor past the end, and inside a multibyte char, are clamped
    let r = comp(Some(&c), "p:am", 999);
    assert_eq!((r.start, r.end), (0, 4));
    let r = comp(Some(&c), "é p:am", 1);
    assert!(r.start <= r.end);
}

#[test]
fn completions_never_panic_and_ranges_are_valid() {
    let c = freq_cache();
    let pieces = ["p:", "a:", "c:", "d:", "is:", "sort:", "-", ",", "\"", "\\", " ", "am", "é", "😀", "x", "/", "home", "this", ":", "=", "lm:"];
    let mut seed: u64 = 99;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..20_000 {
        let n = rnd() % 8;
        let input: String = (0..n).map(|_| pieces[rnd() % pieces.len()]).collect();
        let pos = rnd() % (input.len() + 3);
        for cache in [Some(&c), None] {
            let r = comp(cache, &input, pos);
            assert!(r.start <= r.end && r.end <= input.len(), "{:?} {} -> {}..{}", input, pos, r.start, r.end);
            assert!(input.is_char_boundary(r.start) && input.is_char_boundary(r.end), "{:?} {} -> {}..{}", input, pos, r.start, r.end);
            assert!(r.items.len() <= MAX_ITEMS);
            for i in 0..r.items.len() {
                let _ = apply(&input, &r, i);
            }
        }
    }
}
