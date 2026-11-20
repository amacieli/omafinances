use super::*;
use crate::cache::testing::synthetic;
use crate::db::{RawAccount, RawCategory, RawTxn, Snapshot};
use std::time::Instant;

// Categories (ids): 1 Home(sort 1) > 4 Garden(sort 0), 7 Zeta(sort 0), 2 Improvement(sort 1) > 3 Paint;
// 5 Salary(sort 2) > 6 Job. Tree order: Home, Garden, Zeta, Improvement, Paint, Salary, Job.
// Path order differs: "Home › Zeta" sorts after "Home › Improvement", so category sort is not tree order.
fn cats() -> Vec<RawCategory> {
    let c = |id, parent_id, name: &str, kind: &str, sort| RawCategory { id, parent_id, name: name.into(), kind: kind.into(), sort };
    vec![
        c(1, None, "Home", "expense", 1),
        c(2, Some(1), "Improvement", "expense", 1),
        c(3, Some(2), "Paint", "expense", 0),
        c(4, Some(1), "Garden", "expense", 0),
        c(5, None, "Salary", "income", 2),
        c(6, Some(5), "Job", "income", 0),
        c(7, Some(1), "Zeta", "expense", 0),
    ]
}

fn t(id: i64, date: &str, payee: Option<&str>, orig: Option<&str>, notes: Option<&str>, amount: f64, acct: Option<i64>, cat: Option<i64>) -> RawTxn {
    RawTxn {
        id,
        date: date.into(),
        payee: payee.map(Into::into),
        original_name: orig.map(Into::into),
        notes: notes.map(Into::into),
        amount,
        currency: Some("USD".into()),
        status: Some("cleared".into()),
        source: Some("plaid".into()),
        is_pending: id == 4,
        category_local_id: cat,
        account_id: acct,
        account: acct.map(|a| if a == 1 { "Checking".to_string() } else { "Savings".to_string() }),
        ..Default::default()
    }
}

/// Nine rows, see the expectations in each test. Row 8 has an invalid date; row 5 has no account.
fn fixture() -> Cache {
    let txns = vec![
        t(1, "2026-01-05", Some("Amazon"), Some("AMZN MKTP"), Some("books"), 10.0, Some(1), Some(3)),
        t(2, "2026-01-03", Some("Whole Foods"), Some("WF#123"), None, 50.0, Some(2), Some(4)),
        t(3, "2026-01-05", Some("Payroll"), Some("ACH DEP"), Some("salary"), -1000.0, Some(1), Some(6)),
        t(4, "2026-02-01", Some("Shell"), None, None, 30.0, Some(2), None),
        t(5, "2025-12-31", Some("Coffee"), None, None, 4.5, None, Some(7)),
        t(6, "2026-01-05", Some("amazon"), Some("AMZN REFUND"), None, -20.0, Some(1), Some(3)),
        t(7, "2026-01-07", Some("Rent"), None, None, 1200.0, Some(1), Some(2)),
        t(8, "not-a-date", Some("Broken"), None, None, 1.0, Some(2), None),
        t(9, "2026-01-03", Some("Zero"), None, None, 0.0, Some(2), Some(1)),
    ];
    let accounts = vec![
        RawAccount { id: 1, name: "Chk raw".into(), display_name: None, ..Default::default() },
        RawAccount { id: 2, name: "sav raw".into(), display_name: Some("Savings".into()), ..Default::default() },
    ];
    Cache::build(Snapshot { txns, accounts, categories: cats(), balances: vec![] })
}

fn ids(r: &QueryResult) -> Vec<i64> {
    r.rows.iter().map(|t| t.id).collect()
}

fn q(s: &str) -> Query {
    Query::parse(s).unwrap_or_else(|e| panic!("parse {:?}: {}", s, e))
}

fn go<'a>(c: &'a Cache, s: &str) -> QueryResult<'a> {
    run(c, &q(s))
}

// ---- date and parsing -------------------------------------------------------------------------

#[test]
fn date_keys() {
    assert_eq!(date_key("2026-01-05"), Some(20260105));
    assert_eq!(date_key("2024-02-29"), Some(20240229));
    for bad in ["2025-02-29", "2026-13-01", "2026-00-10", "2026-1-05", "20260105", "2026-01-05 ", "2026/01/05", "", "2026-0a-05"] {
        assert_eq!(date_key(bad), None, "{:?}", bad);
    }
}

#[test]
fn defaults_and_empty_query() {
    assert_eq!(Query::parse("").unwrap(), Query::default());
    assert_eq!(Query::parse("limit=100&offset=0").unwrap(), Query::default());
    let d = Query::default();
    assert_eq!((d.sort, d.desc, d.limit, d.offset), (Sort::Date, true, 100, 0));
    // an empty q is no filter
    assert_eq!(Query::parse("q=").unwrap().q, None);
}

#[test]
fn valid_params_parse() {
    let p = q("sort=amount&dir=asc&account=2&category=none&from=2026-01-01&to=2026-01-31&min=1.5&max=20&flow=in&q=%41mazon%20x&limit=500&offset=7");
    assert_eq!(p.sort, Sort::Amount);
    assert!(!p.desc);
    assert_eq!(p.account, Some(2));
    assert_eq!(p.category, Some(CategoryFilter::Uncategorized));
    assert_eq!((p.from, p.to), (Some(20260101), Some(20260131)));
    assert_eq!((p.min, p.max), (Some(1.5), Some(20.0)));
    assert_eq!(p.flow, Some(Flow::In));
    assert_eq!(p.q.as_deref(), Some("amazon x"));
    assert_eq!((p.limit, p.offset), (500, 7));
    assert_eq!(q("category=-3").category, Some(CategoryFilter::Node(-3)));
    assert_eq!(q("min=-5").min, Some(-5.0));
    assert_eq!(q("min=.5").min, Some(0.5));
}

#[test]
fn bad_params_are_rejected() {
    let bad = [
        "sort=foo",
        "sort=",
        "dir=up",
        "dir=DESC",
        "account=x",
        "account=+1",
        "account=1.5",
        "account=",
        "category=abc",
        "category=",
        "from=2026-02-30",
        "from=2026-1-1",
        "from=",
        "to=20260101",
        "from=2026-02-01&to=2026-01-01",
        "min=abc",
        "min=5.",
        "min=.",
        "min=-",
        "min=nan",
        "min=inf",
        "min=1e3",
        "min=20&max=10",
        "max= 5",
        "flow=both",
        "flow=",
        "limit=0",
        "limit=501",
        "limit=-1",
        "limit=abc",
        "limit=",
        "limit",
        "offset=-1",
        "offset=1.5",
        "offset=",
        "bogus=1",
        "sort=date&unknown=x",
        "limit=5&limit=6",
        "q=%ZZ",
        "q=%4",
        "q=%FF",
        "q=a%",
    ];
    for s in bad {
        assert!(Query::parse(s).is_err(), "should be refused: {}", s);
    }
}

#[test]
fn error_messages_do_not_echo_values() {
    let e = Query::parse("sort=SECRETVALUE").unwrap_err();
    assert!(!e.contains("SECRETVALUE"), "{}", e);
    let e = Query::parse("q=%ZZ").unwrap_err();
    assert!(!e.contains("%ZZ"), "{}", e);
}

// ---- engine -----------------------------------------------------------------------------------

#[test]
fn default_order_is_date_desc_then_id_desc() {
    let c = fixture();
    let r = go(&c, "");
    assert_eq!(r.total, 9);
    assert_eq!(ids(&r), vec![4, 7, 6, 3, 1, 9, 2, 5, 8]);
    assert_eq!(r.sum_out, 1295.5);
    assert_eq!(r.sum_in, 1020.0);
}

#[test]
fn sort_by_date_asc_keeps_id_desc_ties() {
    let c = fixture();
    assert_eq!(ids(&go(&c, "sort=date&dir=asc")), vec![8, 5, 9, 2, 6, 3, 1, 7, 4]);
}

#[test]
fn sort_by_amount_both_directions() {
    let c = fixture();
    assert_eq!(ids(&go(&c, "sort=amount&dir=asc")), vec![3, 6, 9, 8, 5, 1, 4, 2, 7]);
    assert_eq!(ids(&go(&c, "sort=amount&dir=desc")), vec![7, 2, 4, 1, 5, 8, 9, 6, 3]);
}

#[test]
fn sort_by_payee_is_case_insensitive_with_id_tiebreak() {
    let c = fixture();
    // "Amazon" (1) and "amazon" (6) tie: id desc gives 6 first
    assert_eq!(ids(&go(&c, "sort=payee&dir=asc")), vec![6, 1, 8, 5, 3, 7, 4, 2, 9]);
}

#[test]
fn sort_by_account_puts_none_first_ascending() {
    let c = fixture();
    // ranks: none (0) < Checking (1) < Savings (2)
    assert_eq!(ids(&go(&c, "sort=account&dir=asc")), vec![5, 7, 6, 3, 1, 9, 8, 4, 2]);
    assert_eq!(ids(&go(&c, "sort=account&dir=desc")), vec![9, 8, 4, 2, 7, 6, 3, 1, 5]);
}

#[test]
fn sort_by_category_uses_path_not_tree_order() {
    let c = fixture();
    // paths: Home < Home › Garden < Home › Improvement < Home › Improvement › Paint < Home › Zeta < Salary
    // none (0) first ascending; Zeta sits after Paint here though it comes before Improvement in tree order
    assert_eq!(ids(&go(&c, "sort=category&dir=asc")), vec![8, 4, 9, 2, 7, 6, 1, 5, 3]);
}

#[test]
fn category_filter_matches_descendants() {
    let c = fixture();
    // category 2 (Improvement) includes Paint (3)
    assert_eq!(ids(&go(&c, "category=2")), vec![7, 6, 1]);
    // category 1 (Home) includes Garden, Zeta, Improvement and Paint
    let r = go(&c, "category=1");
    assert_eq!(r.total, 6);
    assert_eq!(ids(&r), vec![7, 6, 1, 9, 2, 5]);
    // leaf: Job under Salary; the root Salary gets only its own subtree
    assert_eq!(ids(&go(&c, "category=5")), vec![3]);
    assert_eq!(ids(&go(&c, "category=6")), vec![3]);
    assert_eq!(ids(&go(&c, "category=3")), vec![6, 1]);
}

#[test]
fn category_none_and_unknown() {
    let c = fixture();
    let r = go(&c, "category=none");
    assert_eq!((r.total, ids(&r)), (2, vec![4, 8]));
    assert_eq!(go(&c, "category=999").total, 0);
    assert_eq!(go(&c, "category=-1").total, 0);
}

#[test]
fn account_filter() {
    let c = fixture();
    let r = go(&c, "account=1");
    assert_eq!((r.total, ids(&r)), (4, vec![7, 6, 3, 1]));
    assert_eq!(go(&c, "account=99").total, 0);
    // row 5 has no account and matches no account filter
    assert!(!ids(&go(&c, "account=1")).contains(&5));
}

#[test]
fn date_range_is_inclusive_and_excludes_bad_dates() {
    let c = fixture();
    let r = go(&c, "from=2026-01-05");
    assert_eq!((r.total, ids(&r)), (5, vec![4, 7, 6, 3, 1]));
    let r = go(&c, "to=2026-01-03");
    assert_eq!((r.total, ids(&r)), (3, vec![9, 2, 5]));
    let r = go(&c, "from=2026-01-03&to=2026-01-05");
    assert_eq!((r.total, ids(&r)), (5, vec![6, 3, 1, 9, 2]));
    // from= on a single day
    assert_eq!(ids(&go(&c, "from=2026-01-07&to=2026-01-07")), vec![7]);
}

#[test]
fn flow_splits_by_sign() {
    let c = fixture();
    let r = go(&c, "flow=out");
    assert_eq!(r.total, 6);
    assert_eq!(ids(&r), vec![4, 7, 1, 2, 5, 8]);
    assert_eq!((r.sum_out, r.sum_in), (1295.5, 0.0));
    let r = go(&c, "flow=in");
    assert_eq!((r.total, ids(&r)), (2, vec![6, 3]));
    assert_eq!((r.sum_out, r.sum_in), (0.0, 1020.0));
}

#[test]
fn min_max_compare_the_absolute_amount() {
    let c = fixture();
    assert_eq!(ids(&go(&c, "min=20")), vec![4, 7, 6, 3, 2]);
    assert_eq!(ids(&go(&c, "max=10")), vec![1, 9, 5, 8]);
    assert_eq!(ids(&go(&c, "min=10&max=10")), vec![1]);
    // -20 counts as 20
    assert!(ids(&go(&c, "min=20&max=20")).contains(&6));
}

#[test]
fn free_text_searches_payee_original_and_notes() {
    let c = fixture();
    assert_eq!(ids(&go(&c, "q=amazon")), vec![6, 1]);
    assert_eq!(ids(&go(&c, "q=AMZN")), vec![6, 1]); // original_name
    assert_eq!(ids(&go(&c, "q=books")), vec![1]); // notes
    assert_eq!(ids(&go(&c, "q=salary")), vec![3]); // notes, payee is "Payroll"
    assert_eq!(ids(&go(&c, "q=whole%20foods")), vec![2]);
    assert_eq!(ids(&go(&c, "q=WF%23")), vec![2]);
    assert_eq!(go(&c, "q=nothing-like-this").total, 0);
}

#[test]
fn filters_combine_with_and() {
    let c = fixture();
    let r = go(&c, "account=1&flow=in&sort=amount&dir=asc");
    assert_eq!((r.total, ids(&r)), (2, vec![3, 6]));
    // Home subtree, money out, at least 5, dated on or before 2026-01-03: only Whole Foods
    let r = go(&c, "category=1&flow=out&min=5&to=2026-01-03");
    assert_eq!((r.total, ids(&r)), (1, vec![2]));
}

#[test]
fn paging_and_totals() {
    let c = fixture();
    let r = go(&c, "limit=2&offset=1");
    assert_eq!((r.total, ids(&r)), (9, vec![7, 6]));
    assert_eq!(ids(&go(&c, "offset=8")), vec![8]);
    let r = go(&c, "offset=20");
    assert_eq!((r.total, ids(&r).len()), (9, 0));
    // sums cover every match, not only the page
    let r = go(&c, "limit=1");
    assert_eq!((r.sum_out, r.sum_in), (1295.5, 1020.0));
    // paging is consistent with the full order
    let full = ids(&go(&c, "sort=amount&dir=desc"));
    for off in 0..9 {
        for lim in 1..=9 {
            let got = ids(&go(&c, &format!("sort=amount&dir=desc&limit={}&offset={}", lim, off)));
            let want: Vec<i64> = full.iter().copied().skip(off).take(lim).collect();
            assert_eq!(got, want, "offset {} limit {}", off, lim);
        }
    }
}

#[test]
fn no_match_is_empty_not_error() {
    let c = fixture();
    let r = go(&c, "from=2030-01-01");
    assert_eq!((r.total, r.rows.len(), r.sum_out, r.sum_in), (0, 0, 0.0, 0.0));
}

// ---- brute-force reference on the synthetic 50k set --------------------------------------------

/// Filter and sort the slow, obvious way. Used to check the optimized path on realistic data.
fn reference(c: &Cache, q: &Query) -> Vec<i64> {
    let mut v: Vec<&crate::cache::Txn> = c
        .txns
        .iter()
        .filter(|t| {
            q.account.is_none_or(|a| t.account_id == Some(a))
                && match q.category {
                    None => true,
                    Some(CategoryFilter::Uncategorized) => t.category_id.is_none(),
                    Some(CategoryFilter::Node(id)) => match c.node_pos(id) {
                        None => false,
                        Some(p) => {
                            let node = &c.categories[p as usize];
                            t.category_id.is_some_and(|cid| {
                                let tp = c.node_pos(cid).unwrap();
                                tp >= p && tp < p + node.size
                            })
                        }
                    },
                }
                && q.from.is_none_or(|f| t.date_key != 0 && t.date_key >= f)
                && q.to.is_none_or(|to| t.date_key != 0 && t.date_key <= to)
                && match q.flow {
                    None => true,
                    Some(Flow::Out) => t.amount > 0.0,
                    Some(Flow::In) => t.amount < 0.0,
                }
                && q.min.is_none_or(|m| t.amount.abs() >= m)
                && q.max.is_none_or(|m| t.amount.abs() <= m)
                && q.q.as_ref().is_none_or(|s| {
                    t.payee.to_lowercase().contains(s) || t.original_name.as_deref().unwrap_or("").to_lowercase().contains(s) || t.notes.as_deref().unwrap_or("").to_lowercase().contains(s)
                })
        })
        .collect();
    v.sort_by(|a, b| {
        let ord = match q.sort {
            Sort::Date => a.date_key.cmp(&b.date_key),
            Sort::Amount => a.amount.total_cmp(&b.amount),
            Sort::Payee => a.payee.to_lowercase().cmp(&b.payee.to_lowercase()),
            Sort::Account => a.account.cmp(&b.account),
            Sort::Category => {
                let pa = c.category_path(a);
                let pb = c.category_path(b);
                pa.cmp(&pb)
            }
        };
        let ord = if q.desc { ord.reverse() } else { ord };
        ord.then_with(|| b.id.cmp(&a.id))
    });
    v.iter().skip(q.offset).take(q.limit).map(|t| t.id).collect()
}

#[test]
fn engine_matches_the_reference_on_synthetic_data() {
    let c = Cache::build(synthetic(5_000));
    let cases = [
        "",
        "sort=amount&dir=asc&limit=500",
        "sort=payee&dir=asc&limit=500",
        "sort=account&dir=desc&limit=500",
        "sort=category&dir=asc&limit=500",
        "category=1&sort=date&dir=asc&limit=500",
        "category=2&flow=out&sort=amount&limit=500",
        "category=none&min=10&max=300&limit=500",
        "account=3&from=2024-06-01&to=2025-03-01&limit=500",
        "q=amazon&flow=in&sort=payee&limit=500&offset=30",
        "q=note%201&sort=amount&dir=asc&limit=500",
    ];
    for s in cases {
        let query = q(s);
        let got = ids(&run(&c, &query));
        assert_eq!(got, reference(&c, &query), "case {}", s);
    }
}

// ---- performance ------------------------------------------------------------------------------

/// Load-time cost of building the cache from 50k rows, and one filter+sort query, must stay well
/// under the 100 ms budget even in the debug profile.
#[test]
fn fifty_thousand_rows_filter_and_sort_under_100ms() {
    let snap = synthetic(50_000);
    let t0 = Instant::now();
    let c = Cache::build(snap);
    let build_ms = t0.elapsed().as_millis();
    assert_eq!(c.txns.len(), 50_000);

    let query = q("category=1&flow=out&from=2024-03-01&sort=amount&dir=asc&limit=100&offset=0");
    let mut worst = 0u128;
    for _ in 0..5 {
        let t0 = Instant::now();
        let r = run(&c, &query);
        worst = worst.max(t0.elapsed().as_millis());
        assert!(r.total > 0);
    }
    eprintln!("50k rows: build {} ms, filter+sort worst {} ms (debug)", build_ms, worst);
    assert!(worst < 100, "filter+sort took {} ms", worst);
}

/// Real timing numbers. Run with: cargo test --release -- --ignored --nocapture query_timing
#[test]
#[ignore]
fn query_timing() {
    let snap = synthetic(50_000);
    let t0 = Instant::now();
    let c = Cache::build(snap);
    println!("build 50k rows: {:.2} ms", t0.elapsed().as_secs_f64() * 1e3);

    let cases = [
        ("no filter, date desc, 100", "limit=100"),
        ("no filter, amount asc, 100", "sort=amount&dir=asc&limit=100"),
        ("category subtree + flow, amount", "category=1&flow=out&sort=amount&limit=100"),
        ("from/to + payee sort", "from=2024-03-01&to=2025-06-30&sort=payee&limit=100"),
        ("free text, payee sort, full page", "q=amazon&sort=payee&limit=500"),
        ("account + min/max, category sort", "account=2&min=10&max=250&sort=category&limit=100"),
        ("offset 40000, date asc", "sort=date&dir=asc&offset=40000&limit=100"),
    ];
    for (name, s) in cases {
        let query = q(s);
        let mut times = Vec::new();
        let mut total = 0;
        for _ in 0..50 {
            let t0 = Instant::now();
            let r = run(&c, &query);
            times.push(t0.elapsed().as_secs_f64() * 1e3);
            total = r.total;
        }
        times.sort_by(|a, b| a.total_cmp(b));
        println!(
            "{:<36} matched {:>6}  median {:.3} ms  min {:.3} ms  max {:.3} ms",
            name,
            total,
            times[times.len() / 2],
            times[0],
            times[times.len() - 1]
        );
    }
}
