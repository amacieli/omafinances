use super::fixture::{fixture, today};
use super::*;
use crate::cache::testing::synthetic;
use crate::cache::Cache;
use crate::query::{run_with, Query};
use chrono::NaiveDate;
use std::time::Instant;

fn p(s: &str) -> Search {
    parse(s, today()).unwrap_or_else(|e| panic!("parse {:?}: {}", s, e))
}

fn ids_in(c: &Cache, q: &str) -> Vec<i64> {
    let comp = p(q).compile(c);
    let mut v: Vec<i64> = c.txns.iter().filter(|t| comp.matches(t)).map(|t| t.id).collect();
    v.sort();
    v
}

fn ids(q: &str) -> Vec<i64> {
    ids_in(&fixture(), q)
}

#[test]
fn free_text_and_payee_matching() {
    assert_eq!(ids("amazon"), vec![1, 6]);
    assert_eq!(ids("AMZN"), vec![1, 6]); // original_name
    assert_eq!(ids("books"), vec![1]); // notes
    assert_eq!(ids("\"whole foods\""), vec![2]);
    assert_eq!(ids("whole foods"), vec![2]); // two words, ANDed
    assert_eq!(ids("whole amazon"), Vec::<i64>::new());
    assert_eq!(ids("p:amz"), vec![1, 6]); // original name counts for p:
    assert_eq!(ids("p:books"), Vec::<i64>::new()); // notes do not
    assert_eq!(ids("p:amazon,rent"), vec![1, 6, 7]);
    assert_eq!(ids("p:AMAZON"), vec![1, 6]);
    assert_eq!(ids(""), (1..=9).collect::<Vec<_>>());
}

#[test]
fn account_note_lm_and_id() {
    assert_eq!(ids("a:sav"), vec![2, 4, 8, 9]);
    assert_eq!(ids("acct:chk"), Vec::<i64>::new()); // display name is "Checking"
    assert_eq!(ids("account:check"), vec![1, 3, 6, 7]);
    assert_eq!(ids("a:checking,savings"), vec![1, 2, 3, 4, 6, 7, 8, 9]); // row 5 has no account
    assert_eq!(ids("n:book"), vec![1]);
    assert_eq!(ids("notes:books,salary"), vec![1, 3]);
    assert_eq!(ids("lm:shopping"), vec![1, 6]);
    assert_eq!(ids("lm:shop"), vec![1, 5, 6]);
    assert_eq!(ids("id:1,2,3"), vec![1, 2, 3]);
    assert_eq!(ids("id:7"), vec![7]);
    assert_eq!(ids("id:404"), Vec::<i64>::new());
}

#[test]
fn categories_descendants_none_and_prefix_segments() {
    // Home(1) > Improvement(2) > Paint(3); Home > Garden(4); Home > Zeta(7); Salary(5) > Job(6)
    assert_eq!(ids("c:home"), vec![1, 2, 5, 6, 7, 9]); // Home and every descendant
    assert_eq!(ids("c:Home"), ids("c:home"));
    assert_eq!(ids("category:ho"), ids("c:home")); // prefix per segment
    assert_eq!(ids("c:home/imp"), vec![1, 6, 7]); // Improvement and Paint
    assert_eq!(ids("c:home/improvement/paint"), vec![1, 6]);
    assert_eq!(ids("c:home/imp/pa"), vec![1, 6]);
    assert_eq!(ids("c:HOME/IMP"), vec![1, 6, 7]);
    assert_eq!(ids("c:home/garden"), vec![2]);
    assert_eq!(ids("c:home/nothing"), Vec::<i64>::new());
    assert_eq!(ids("c:salary"), vec![3]);
    assert_eq!(ids("c:sal/jo"), vec![3]);
    assert_eq!(ids("c:job"), Vec::<i64>::new()); // paths start at the root
    assert_eq!(ids("c:none"), vec![4, 8]);
    assert_eq!(ids("c:none,salary"), vec![3, 4, 8]);
    assert_eq!(ids("-c:none"), vec![1, 2, 3, 5, 6, 7, 9]);
    assert_eq!(ids("c:\"home/improvement\""), vec![1, 6, 7]);
}

#[test]
fn dates_with_a_fixed_today() {
    assert_eq!(ids("d:2026-01-05"), vec![1, 3, 6]);
    assert_eq!(ids("d:2026-01"), vec![1, 2, 3, 6, 7, 9]);
    assert_eq!(ids("d:2026"), vec![1, 2, 3, 4, 6, 7, 9]); // row 8 has no valid date: never matches
    assert_eq!(ids("d:2026-01-03..2026-01-05"), vec![1, 2, 3, 6, 9]);
    assert_eq!(ids("d:2026-01-06.."), vec![4, 7]);
    assert_eq!(ids("d:..2026-01-03"), vec![2, 5, 9]);
    assert_eq!(ids("d:>2026-01-05"), vec![4, 7]);
    assert_eq!(ids("d:>=2026-01-05"), vec![1, 3, 4, 6, 7]);
    assert_eq!(ids("d:<2026-01-03"), vec![5]);
    assert_eq!(ids("d:<=2026-01-03"), vec![2, 5, 9]);
    assert_eq!(ids("d:2026-01-05,2025-12"), vec![1, 3, 5, 6]);
    // an invalid stored date does not match even an open range; its negation does
    assert!(!ids("d:..2099").contains(&8));
    assert!(ids("-d:2026").contains(&8));
}

#[test]
fn preset_and_relative_dates() {
    // today = Thursday 2026-01-08
    assert_eq!(ids("d:today"), Vec::<i64>::new());
    assert_eq!(ids("d:yesterday"), vec![7]);
    assert_eq!(ids("d:this-week"), vec![1, 3, 6, 7]); // Mon 01-05 .. Sun 01-11
    assert_eq!(ids("d:last-week"), vec![2, 5, 9]); // 12-29 .. 01-04
    assert_eq!(ids("d:this-month"), vec![1, 2, 3, 6, 7, 9]);
    assert_eq!(ids("d:last-month"), vec![5]);
    assert_eq!(ids("d:this-quarter"), vec![1, 2, 3, 4, 6, 7, 9]);
    assert_eq!(ids("d:last-quarter"), vec![5]);
    assert_eq!(ids("d:ytd"), vec![1, 2, 3, 6, 7, 9]); // 02-01 is after today
    assert_eq!(ids("d:this-year"), vec![1, 2, 3, 4, 6, 7, 9]);
    assert_eq!(ids("d:last-year"), vec![5]);
    assert_eq!(ids("d:7d"), vec![1, 2, 3, 6, 7, 9]); // 01-02 .. 01-08, not the February row
    assert_eq!(ids("d:5d"), vec![1, 3, 6, 7]); // 01-04 .. 01-08
    assert_eq!(ids("d:>7d"), vec![1, 2, 3, 4, 6, 7, 9]); // newer than 7 days ago; no upper bound
    assert_eq!(ids("d:>=5d"), vec![1, 2, 3, 4, 6, 7, 9]);
    assert_eq!(ids("d:<5d"), vec![5]); // older than 01-03
    assert_eq!(ids("d:<=5d"), vec![2, 5, 9]);
    assert_eq!(ids("d:1m"), vec![1, 2, 3, 5, 6, 7, 9]);
    assert_eq!(ids("d:1y"), vec![1, 2, 3, 5, 6, 7, 9]);
    // other "today" values move the answer
    let c = fixture();
    let s = parse("d:this-month", NaiveDate::from_ymd_opt(2026, 2, 14).unwrap()).unwrap();
    let comp = s.compile(&c);
    assert_eq!(c.txns.iter().filter(|t| comp.matches(t)).map(|t| t.id).collect::<Vec<_>>(), vec![4]);
}

#[test]
fn amounts_compare_the_absolute_value() {
    // 1: 10, 2: 50, 3: -1000, 4: 30, 5: 4.5, 6: -20, 7: 1200, 8: 1, 9: 0
    assert_eq!(ids("amount:>50"), vec![3, 7]);
    assert_eq!(ids(">50"), vec![3, 7]);
    assert_eq!(ids(">=50"), vec![2, 3, 7]);
    assert_eq!(ids("<5"), vec![5, 8, 9]);
    assert_eq!(ids("<=4.5"), vec![5, 8, 9]);
    assert_eq!(ids("10..50"), vec![1, 2, 4, 6]);
    assert_eq!(ids("amount:..10"), vec![1, 5, 8, 9]);
    assert_eq!(ids("amount:1000.."), vec![3, 7]);
    assert_eq!(ids("1000"), vec![3]); // exact, sign ignored
    assert_eq!(ids("=20"), vec![6]);
    assert_eq!(ids("100"), Vec::<i64>::new());
    assert_eq!(ids("amount:4.50"), vec![5]);
    assert_eq!(ids(">$1,200"), Vec::<i64>::new());
    assert_eq!(ids(">=$1,200"), vec![7]);
    assert_eq!(ids("amount:>$1,000"), vec![7]);
    assert_eq!(ids("amount:1,200"), vec![7]);
    assert_eq!(ids("amount:10,20"), vec![1, 6]);
    assert_eq!(ids("amount:>1000,<2"), vec![7, 8, 9]);
    assert_eq!(ids("amt:0"), vec![9]);
    assert_eq!(ids("-20"), vec![1, 2, 3, 4, 5, 7, 8, 9]); // negation of amount 20
}

#[test]
fn is_flags() {
    assert_eq!(ids("is:recurring"), vec![7]);
    assert_eq!(ids("is:pending"), vec![4]);
    assert_eq!(ids("is:transfer"), vec![8, 9]); // case-insensitive over the LM category
    assert_eq!(ids("is:income"), vec![3, 6]);
    assert_eq!(ids("is:expense"), vec![1, 2, 4, 5, 7, 8]); // zero is neither
    assert_eq!(ids("is:uncat"), vec![4, 8]);
    assert_eq!(ids("is:categorized"), vec![1, 2, 3, 5, 6, 7, 9]);
    assert_eq!(ids("is:income,pending"), vec![3, 4, 6]);
    assert_eq!(ids("is:uncat is:expense"), vec![4, 8]);
}

#[test]
fn negation_and_combinations() {
    assert_eq!(ids("-p:amazon"), vec![2, 3, 4, 5, 7, 8, 9]);
    assert_eq!(ids("-amazon"), vec![2, 3, 4, 5, 7, 8, 9]);
    assert_eq!(ids("p:amazon -is:income"), vec![1]);
    assert_eq!(ids("-is:expense -is:income"), vec![9]);
    assert_eq!(ids("c:home amount:>5 d:2026-01"), vec![1, 2, 6, 7]);
    assert_eq!(ids("c:home -c:home/imp"), vec![2, 5, 9]);
    assert_eq!(ids("a:sav is:expense -c:none"), vec![2]);
    assert_eq!(ids("p:amazon,rent,payroll -a:chk"), vec![1, 3, 6, 7]); // "chk" is not in "checking"
    assert_eq!(ids("p:amazon,rent,payroll -a:check"), Vec::<i64>::new());
}

#[test]
fn sort_directive_is_not_a_filter() {
    assert_eq!(ids("sort:-amount"), (1..=9).collect::<Vec<_>>());
    assert_eq!(ids("p:amazon sort:amount"), vec![1, 6]);
}

#[test]
fn composes_with_structured_filters() {
    let c = fixture();
    let run = |structured: &str, search: &str| -> Vec<i64> {
        let q = Query::parse(structured).unwrap();
        let comp = p(search).compile(&c);
        let pred = |t: &crate::cache::Txn| comp.matches(t);
        let r = run_with(&c, &q, Some(&pred));
        r.rows.iter().map(|t| t.id).collect()
    };
    // structured only / search only / both
    let mut out = run("flow=out", "");
    out.sort();
    assert_eq!(out, vec![1, 2, 4, 5, 7, 8]);
    assert_eq!(run("flow=out&sort=amount&dir=asc", "p:amazon"), vec![1]);
    assert_eq!(run("flow=in", "p:amazon"), vec![6]);
    assert_eq!(run("from=2026-01-05&sort=amount&dir=asc", "c:home"), vec![6, 1, 7]);
    assert_eq!(run("min=40&sort=amount&dir=asc", "a:sav"), vec![2]);
    assert_eq!(run("q=amazon", "is:income"), vec![6]);
    assert_eq!(run("account=1", "d:2026-01-05 -is:income"), vec![1]);
    // totals follow the combined filter
    let q = Query::parse("flow=out").unwrap();
    let comp = p("c:home").compile(&c);
    let pred = |t: &crate::cache::Txn| comp.matches(t);
    let r = run_with(&c, &q, Some(&pred));
    assert_eq!(r.total, 4);
    assert!((r.sum_out - (10.0 + 50.0 + 4.5 + 1200.0)).abs() < 1e-9);
}

// ---- performance ------------------------------------------------------------------------------

const TYPICAL: &str = "p:amazon c:home d:this-year";

fn timed(c: &Cache, today: NaiveDate, query: &str) -> (f64, usize) {
    let q = Query::parse("limit=100").unwrap();
    let t0 = Instant::now();
    let s = parse(query, today).unwrap();
    let comp = s.compile(c);
    let pred = |t: &crate::cache::Txn| comp.matches(t);
    let r = run_with(c, &q, Some(&pred));
    (t0.elapsed().as_secs_f64() * 1e3, r.total)
}

#[test]
fn typical_search_over_fifty_thousand_rows_is_under_100ms() {
    let c = Cache::build(synthetic(50_000));
    let today = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
    let mut worst: f64 = 0.0;
    let mut total = 0;
    for _ in 0..5 {
        let (ms, n) = timed(&c, today, TYPICAL);
        worst = worst.max(ms);
        total = n;
    }
    assert!(total > 0);
    eprintln!("50k rows, {:?}: worst {:.2} ms (debug), {} matches", TYPICAL, worst, total);
    assert!(worst < 100.0, "search took {} ms", worst);
}

/// Real timing numbers. Run with: cargo test --release -- --ignored --nocapture search_timing
#[test]
#[ignore]
fn search_timing() {
    let c = Cache::build(synthetic(50_000));
    let today = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
    for query in [
        TYPICAL,
        "amazon",
        "p:amazon,costco -is:income amount:>20",
        "c:home/sub amount:10..200 d:>90d",
        "a:account -c:none is:expense sort:-amount",
        "\"whole foods\" n:x",
    ] {
        let mut runs: Vec<f64> = Vec::new();
        let mut n = 0;
        for _ in 0..20 {
            let (ms, total) = timed(&c, today, query);
            runs.push(ms);
            n = total;
        }
        runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("{:<48} median {:6.2} ms  worst {:6.2} ms  matches {}", query, runs[runs.len() / 2], runs[runs.len() - 1], n);
    }
}
