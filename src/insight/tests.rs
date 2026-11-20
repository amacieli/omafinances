//! Golden tests on synthetic, realistic data. Today is pinned to 2026-11-20.

use super::*;
use crate::cache::Cache;
use crate::db::rules::RuleRow;
use crate::db::{RawAccount, RawCategory, RawTxn, Snapshot};
use crate::smart::normalize::Aliases;
use chrono::{Days, Months, NaiveDate};
use serde_json::{json, Value};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn ds(x: NaiveDate) -> String {
    x.format("%Y-%m-%d").to_string()
}

fn tx(id: i64, date: &str, payee: &str, amount: f64, acct: i64, cat: Option<i64>) -> RawTxn {
    RawTxn {
        id,
        date: date.to_string(),
        payee: Some(payee.to_string()),
        original_name: None,
        notes: None,
        amount,
        currency: Some("USD".into()),
        status: None,
        source: None,
        tag_ids: None,
        recurring_id: None,
        external_id: None,
        created_at_lm: None,
        updated_at_lm: None,
        is_pending: false,
        category_id: None,
        category_local_id: cat,
        lm_category: None,
        account_id: Some(acct),
        account: Some(["", "Checking", "Savings", "Card"][acct as usize].to_string()),
    }
}

fn cats(n: i64) -> Vec<RawCategory> {
    (1..=n)
        .map(|i| RawCategory { id: i, parent_id: None, name: format!("Cat {i:02}"), kind: "expense".into(), sort: i })
        .collect()
}

fn cache_of(txns: Vec<RawTxn>, ncats: i64) -> Cache {
    let accounts = (1..=3)
        .map(|a| RawAccount { id: a, name: ["", "Checking", "Savings", "Card"][a as usize].into(), ..Default::default() })
        .collect();
    Cache::build(Snapshot { txns, accounts, categories: cats(ncats), balances: vec![] })
}

fn model_of(c: &Cache) -> Model {
    Model::build(c, &Aliases::none(), &[])
}

fn lcg(seed: &mut u64) -> u32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*seed >> 33) as u32
}

fn plus_month(x: NaiveDate) -> NaiveDate {
    x.checked_add_months(Months::new(1)).unwrap()
}

/// `n` rows of one payee in account 1; `next` gives the following date.
fn series(
    out: &mut Vec<RawTxn>,
    id: &mut i64,
    payee: &str,
    start: &str,
    n: usize,
    next: impl Fn(NaiveDate, usize) -> NaiveDate,
    amount: impl Fn(usize) -> f64,
) {
    let mut date = d(start);
    for k in 0..n {
        out.push(tx(*id, &ds(date), payee, amount(k), 1, None));
        *id += 1;
        date = next(date, k);
    }
}

// ---- recurring --------------------------------------------------------------------------------

#[test]
fn recurring_detects_periods_and_skips_noise() {
    let mut v = Vec::new();
    let mut id = 1;
    // monthly, 14 occurrences ending 2026-11-15 (active)
    series(&mut v, &mut id, "NETFLIX.COM", "2025-10-15", 14, |dt, _| plus_month(dt), |_| 15.49);
    // weekly, 11 occurrences, last 2026-11-16
    series(&mut v, &mut id, "Corner Gym", "2026-09-07", 11, |dt, _| dt + Days::new(7), |k| 12.0 + (k % 2) as f64 * 0.5);
    // yearly, 3 occurrences with a few days of jitter; last 2026-03-10
    for y in ["2024-03-12", "2025-03-09", "2026-03-10"] {
        v.push(tx(id, y, "Prime Annual", 139.0, 1, None));
        id += 1;
    }
    // jittery monthly gaps of 28..33 days
    let gaps = [30, 28, 32, 31, 29, 33, 30];
    let mut dt = d("2026-01-02");
    for g in gaps.iter().chain(std::iter::once(&30)) {
        v.push(tx(id, &ds(dt), "Landlord LLC", 1800.0, 1, None));
        id += 1;
        dt = dt + Days::new(*g as u64);
    }
    // inactive monthly: stopped 2026-02
    series(&mut v, &mut id, "Old Magazine", "2025-09-05", 6, |dt, _| plus_month(dt), |_| 6.0);
    // monthly timing but wildly varying amounts: not recurring
    series(&mut v, &mut id, "Pharmacy", "2026-05-01", 6, |dt, _| plus_month(dt), |k| [5.0, 80.0, 12.0, 150.0, 9.0, 60.0][k]);
    // two occurrences only; irregular gaps
    series(&mut v, &mut id, "Rare Shop", "2026-01-01", 2, |dt, _| dt + Days::new(30), |_| 20.0);
    series(&mut v, &mut id, "Random Cafe", "2026-01-01", 6, |dt, k| dt + Days::new([3, 40, 9, 77, 5][k % 5]), |_| 6.0);
    // monthly income (negative amounts)
    series(&mut v, &mut id, "ACME PAYROLL", "2026-01-31", 10, |dt, _| plus_month(dt), |_| -2500.0);

    let c = cache_of(v, 1);
    let m = model_of(&c);
    let s = recurring::detect(&c, &m, today());
    let names: Vec<(&str, &str, bool)> = s.iter().map(|x| (x.payee.as_str(), x.period, x.active)).collect();
    assert_eq!(
        names,
        vec![
            ("acme payroll", "monthly", true),
            ("corner gym", "weekly", true),
            ("netflix", "monthly", true),
            ("prime annual", "yearly", true),
            ("landlord llc", "monthly", false),
            ("old magazine", "monthly", false),
        ]
    );
    let nf = s.iter().find(|x| x.payee == "netflix").unwrap();
    assert_eq!((nf.count, nf.period_days, nf.amount_median, nf.amount_cv), (14, 30, 15.49, 0.0));
    assert_eq!((nf.last_date.as_str(), nf.next_expected.as_str()), ("2026-11-15", "2026-12-15"));
    assert_eq!(nf.account.as_deref(), Some("Checking"));
    let gym = s.iter().find(|x| x.payee == "corner gym").unwrap();
    assert_eq!((gym.count, gym.next_expected.as_str(), gym.period_days), (11, "2026-11-23", 7));
    let yr = s.iter().find(|x| x.payee == "prime annual").unwrap();
    assert_eq!(yr.next_expected, "2027-03-10");
    let pay = s.iter().find(|x| x.payee == "acme payroll").unwrap();
    assert_eq!(pay.amount_median, -2500.0);
    let ll = s.iter().find(|x| x.payee == "landlord llc").unwrap();
    assert_eq!(ll.count, 8);
    let old = s.iter().find(|x| x.payee == "old magazine").unwrap();
    assert!(!old.active);
    assert!(!ll.active, "last 2026-08-03 is > 1.5 periods before today");
    let j = recurring::report(&s);
    assert_eq!(j["series"][2]["payee"], "netflix");
    assert_eq!(j["series"][2]["txn_ids"].as_array().unwrap().len(), 14);
    assert_eq!(j["series"][2]["period"], "monthly");
}

// ---- anomalies --------------------------------------------------------------------------------

#[test]
fn anomalies_spike_duplicate_new_payee_and_transfer_exclusion() {
    let mut v = Vec::new();
    let priors = [40.0, 41.0, 42.10, 43.0, 44.0, 42.10];
    for (k, a) in priors.iter().enumerate() {
        v.push(tx(1 + k as i64, &format!("2026-0{}-10", k + 1), "SHELL OIL 5512", *a, 1, None));
    }
    v.push(tx(10, "2026-08-10", "SHELL OIL 5512", 143.14, 1, None)); // 3.4x
    v.push(tx(11, "2026-09-10", "SHELL OIL 5512", 80.0, 1, None)); // 1.9x: not a spike
    // too few priors: never a spike
    for k in 0..3 {
        v.push(tx(20 + k, &format!("2026-0{}-05", k + 1), "Tiny Cafe", 5.0, 1, None));
    }
    v.push(tx(23, "2026-04-05", "Tiny Cafe", 500.0, 1, None));
    // duplicates: 2 days apart same account (flag the later), other account (no), 5 days apart (no)
    v.push(tx(30, "2026-06-01", "Lunch Place", 12.50, 1, None));
    v.push(tx(31, "2026-06-03", "Lunch Place", 12.50, 1, None));
    v.push(tx(32, "2026-06-03", "Lunch Place", 12.50, 2, None));
    v.push(tx(33, "2026-06-20", "Bakery", 8.0, 1, None));
    v.push(tx(34, "2026-06-25", "Bakery", 8.0, 1, None));
    // new payees: recent and big, recent and small, old and big, seen before
    v.push(tx(40, "2026-11-15", "Brand New Store", 120.0, 1, None));
    v.push(tx(41, "2026-11-16", "Small New Store", 20.0, 1, None));
    v.push(tx(42, "2026-05-01", "Old Big Store", 300.0, 1, None));
    v.push(tx(43, "2026-11-10", "Old Big Store", 300.0, 1, None));
    // a transfer pair that would otherwise be a new payee and a duplicate
    v.push(tx(50, "2026-11-12", "Move Money", 2000.0, 1, None));
    v.push(tx(51, "2026-11-12", "Move Money", -2000.0, 2, None));
    let c = cache_of(v, 1);
    let m = model_of(&c);
    let items = anomalies::detect(&c, &m, today());
    let got: Vec<(&str, i64)> = items.iter().map(|i| (i.kind, c.txns[i.idx].id)).collect();
    assert_eq!(got, vec![("new_payee", 40), ("amount_spike", 10), ("duplicate", 31)]);
    let spike = items.iter().find(|i| i.kind == "amount_spike").unwrap();
    assert_eq!(spike.detail, "3.4x this payee's median of $42.10");
    let j = anomalies::report(&c, &items);
    assert_eq!(j["items"][0]["payee"], "Brand New Store");
    assert_eq!(j["items"][1]["amount"], 143.14);
}

#[test]
fn anomalies_cap_at_200_newest_first() {
    let mut v = Vec::new();
    for k in 0..250i64 {
        let date = ds(d("2025-01-01") + Days::new(k as u64));
        v.push(tx(k * 2 + 1, &date, "Dup Payee", 9.99, 1, None));
        v.push(tx(k * 2 + 2, &date, "Dup Payee", 9.99, 1, None));
    }
    let c = cache_of(v, 1);
    let m = model_of(&c);
    let items = anomalies::detect(&c, &m, today());
    assert_eq!(items.len(), 200);
    assert_eq!(c.txns[items[0].idx].date, "2025-09-07");
}

// ---- transfers --------------------------------------------------------------------------------

#[test]
fn transfers_pair_one_to_one_with_ties_and_non_pairs() {
    let v = vec![
        // plain pair, 1 day apart
        tx(1, "2026-10-01", "To Savings", 500.0, 1, None),
        tx(2, "2026-10-02", "From Checking", -500.0, 2, None),
        // tie: two equal outs the same day, one in: the lowest out id wins
        tx(3, "2026-10-05", "x", 75.0, 1, None),
        tx(4, "2026-10-05", "x", 75.0, 1, None),
        tx(5, "2026-10-05", "y", -75.0, 2, None),
        // smallest gap wins: out 6 is 3 days from in 7 and 1 day from in 8
        tx(6, "2026-10-10", "z", 20.0, 1, None),
        tx(7, "2026-10-13", "z", -20.0, 3, None),
        tx(8, "2026-10-11", "z", -20.0, 3, None),
        // same account: never a pair
        tx(9, "2026-10-15", "same", 30.0, 1, None),
        tx(10, "2026-10-15", "same", -30.0, 1, None),
        // five days apart: no pair
        tx(11, "2026-10-20", "far", 60.0, 1, None),
        tx(12, "2026-10-25", "far", -60.0, 2, None),
        // a cent apart: no pair
        tx(13, "2026-10-28", "cent", 10.00, 1, None),
        tx(14, "2026-10-28", "cent", -10.01, 2, None),
    ];
    let c = cache_of(v, 1);
    let tr = transfers::detect(&c);
    let pairs: Vec<(i64, i64, i32)> = tr.pairs.iter().map(|p| (c.txns[p.out].id, c.txns[p.inn].id, p.days)).collect();
    // newest first by the later row's date, then out id
    assert_eq!(pairs, vec![(6, 8, 1), (3, 5, 0), (1, 2, 1)]);
    let used: Vec<i64> = c.txns.iter().enumerate().filter(|(i, _)| tr.is_transfer[*i]).map(|(_, t)| t.id).collect();
    assert_eq!(used, vec![1, 2, 3, 5, 6, 8]);
    let j = transfers::report(&c, &tr);
    assert_eq!(j["count"], 3);
    assert_eq!(j["total"], 595.0);
    assert_eq!(j["pairs"][2]["out_account"], "Checking");
    assert_eq!(j["pairs"][2]["in_account"], "Savings");
    assert_eq!(j, transfers::report(&c, &transfers::detect(&c)), "deterministic");
}

// ---- dashboard --------------------------------------------------------------------------------

fn dash(c: &Cache, months: u32, excl: bool) -> Value {
    let m = model_of(c);
    dashboard::report(c, &m, today(), &dashboard::Params { months, exclude_transfers: excl })
}

#[test]
fn dashboard_zero_fills_months_and_excludes_transfers() {
    let v = vec![
        tx(1, "2026-08-03", "Grocer", 100.0, 1, Some(1)),
        tx(2, "2026-08-20", "Employer", -1000.0, 1, None),
        tx(3, "2026-11-02", "Grocer", 50.25, 1, Some(1)),
        tx(4, "2026-11-03", "Mystery", 10.0, 1, None),
        tx(5, "2026-11-04", "Cafe", 5.0, 1, Some(2)),
        tx(6, "2026-07-31", "Too Old", 999.0, 1, Some(1)),
        // transfer pair in November
        tx(7, "2026-11-10", "To Savings", 400.0, 1, Some(2)),
        tx(8, "2026-11-10", "From Checking", -400.0, 2, None),
        // after the window
        tx(9, "2026-12-01", "Later", 77.0, 1, Some(1)),
    ];
    let c = cache_of(v, 2);
    let j = dash(&c, 4, true);
    assert_eq!((j["from"].as_str(), j["to"].as_str()), (Some("2026-08-01"), Some("2026-11-30")));
    let months: Vec<(&str, f64, f64, f64)> = j["months"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["month"].as_str().unwrap(), m["out"].as_f64().unwrap(), m["in"].as_f64().unwrap(), m["net"].as_f64().unwrap()))
        .collect();
    assert_eq!(
        months,
        vec![("2026-08", 100.0, 1000.0, 900.0), ("2026-09", 0.0, 0.0, 0.0), ("2026-10", 0.0, 0.0, 0.0), ("2026-11", 65.25, 0.0, -65.25)]
    );
    assert_eq!(j["totals"], json!({ "out": 165.25, "in": 1000.0, "net": 834.75 }));
    assert_eq!(j["transfers_excluded"], json!({ "pairs": 1, "total": 400.0 }));
    let cats: Vec<(Value, &str, f64, u64)> = j["by_category"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["category_id"].clone(), c["path"].as_str().unwrap(), c["out"].as_f64().unwrap(), c["count"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        cats,
        vec![(json!(1), "Cat 01", 150.25, 2), (json!(2), "Cat 02", 5.0, 1), (Value::Null, "Uncategorized", 10.0, 1)]
    );
    let payees: Vec<(&str, f64)> =
        j["top_payees"].as_array().unwrap().iter().map(|p| (p["payee"].as_str().unwrap(), p["out"].as_f64().unwrap())).collect();
    assert_eq!(payees, vec![("grocer", 150.25), ("mystery", 10.0), ("cafe", 5.0)]);

    let k = dash(&c, 4, false);
    assert_eq!(k["totals"], json!({ "out": 565.25, "in": 1400.0, "net": 834.75 }));
    assert_eq!(k["transfers_excluded"], json!({ "pairs": 0, "total": 0.0 }));
    let one = dash(&c, 1, true);
    assert_eq!(one["months"].as_array().unwrap().len(), 1);
    assert_eq!(one["from"], "2026-11-01");
}

#[test]
fn dashboard_groups_into_other_and_uncategorized() {
    let mut v = Vec::new();
    for cat in 1..=17i64 {
        v.push(tx(cat, "2026-11-05", &format!("Shop{}", (b'a' + cat as u8) as char), cat as f64 * 10.0, 1, Some(cat)));
    }
    v.push(tx(100, "2026-11-06", "Mystery", 3.0, 1, None));
    let c = cache_of(v, 17);
    let j = dash(&c, 1, true);
    let rows = j["by_category"].as_array().unwrap();
    assert_eq!(rows.len(), 17, "15 + Other + Uncategorized");
    assert_eq!(rows[0]["path"], "Cat 17");
    assert_eq!(rows[14]["path"], "Cat 03");
    // categories 1 and 2 roll into Other: 10 + 20
    assert_eq!((rows[15]["path"].as_str(), rows[15]["out"].as_f64(), rows[15]["count"].as_u64()), (Some("Other"), Some(30.0), Some(2)));
    assert!(rows[15]["category_id"].is_null());
    assert_eq!((rows[16]["path"].as_str(), rows[16]["out"].as_f64()), (Some("Uncategorized"), Some(3.0)));
    assert_eq!(j["top_payees"].as_array().unwrap().len(), 10);
}

// ---- suggest ----------------------------------------------------------------------------------

/// `n` labeled rows over `k` categories; each category has its own merchant names and typical amount.
fn separable(n: usize, k: i64, seed: u64) -> Vec<RawTxn> {
    let mut s = seed;
    let mut v = Vec::new();
    for i in 0..n {
        let cat = (i as i64 % k) + 1;
        let payee = format!("Merchant{} Store {}", (b'a' + cat as u8) as char, lcg(&mut s) % 3);
        let amount = (cat as f64) * 20.0 + (lcg(&mut s) % 500) as f64 / 100.0;
        let day = 1 + lcg(&mut s) % 28;
        let month = 1 + lcg(&mut s) % 9;
        v.push(tx(i as i64 + 1, &format!("2026-0{month}-{day:02}"), &payee, amount, 1, Some(cat)));
    }
    v
}

fn rule(id: i64, payee: &str, cat: i64) -> RuleRow {
    RuleRow { id, position: id, name: None, enabled: true, matched: json!({ "payee_norm": payee }), category_id: Some(cat), category_path: None, hits: 0 }
}

#[test]
fn suggest_gate_disabled_below_30_labeled() {
    let c = cache_of(separable(29, 4, 7), 4);
    let m = model_of(&c);
    let s = m.suggest(&c);
    assert!(!s.enabled);
    assert_eq!(s.reason.as_deref(), Some("only 29 labeled rows (need 30)"));
    assert_eq!(s.accuracy.labeled, 29);
    assert_eq!(s.accuracy.folds, 5);
    assert!(s.accuracy.top3 > 0.0, "accuracy is still measured");
    assert!(suggest::options(&m, &c, 0, true).iter().all(|o| o.source == "rule"));
    // 30 separable rows open the gate
    let c = cache_of(separable(30, 4, 7), 4);
    let m = model_of(&c);
    assert!(m.suggest(&c).enabled);
}

#[test]
fn suggest_enabled_on_separable_set_and_cv_is_deterministic() {
    let mut v = separable(200, 6, 11);
    for k in 0..6i64 {
        v.push(tx(1000 + k, "2026-10-04", &format!("Merchant{} Store 1", (b'a' + (k + 1) as u8) as char), (k + 1) as f64 * 20.0 + 1.0, 1, None));
    }
    let c = cache_of(v, 6);
    let m = model_of(&c);
    let s = m.suggest(&c);
    assert!(s.enabled, "{:?}", s.reason);
    assert_eq!(s.reason, None);
    assert!(s.accuracy.top1 >= 0.95 && s.accuracy.top3 >= 0.99, "{:?}", s.accuracy);
    assert_eq!(s.accuracy.labeled, 200);
    let again = suggest::Suggest::train(&c, &m.norms);
    assert_eq!(again.accuracy, s.accuracy, "CV is bit-identical across runs");
    for k in 0..6i64 {
        let i = c.txns.binary_search_by_key(&(1000 + k), |t| t.id).unwrap();
        let o = suggest::options(&m, &c, i, true);
        assert_eq!(o[0].category_id, k + 1, "row {k}");
        assert_eq!(o[0].source, "model");
        assert!(o[0].confidence >= 0.3 && o.len() <= 3);
        assert_eq!(o[0].path, format!("Cat {:02}", k + 1));
    }
}

#[test]
fn suggest_gate_disabled_on_noise_with_reason() {
    let mut s = 99u64;
    let mut v = Vec::new();
    for i in 0..120i64 {
        let cat = 1 + (lcg(&mut s) % 10) as i64;
        let payee = format!("Shop{}", lcg(&mut s) % 40);
        v.push(tx(i + 1, "2026-03-10", &payee, 10.0, 1, Some(cat)));
    }
    let c = cache_of(v, 10);
    let m = model_of(&c);
    let g = m.suggest(&c);
    assert!(!g.enabled);
    let reason = g.reason.clone().unwrap();
    assert!(reason.starts_with("top-3 accuracy 0.") && reason.ends_with(" below 0.70"), "{reason}");
    assert!(suggest::options(&m, &c, 0, true).is_empty());
}

#[test]
fn suggest_rule_options_come_first_and_are_not_duplicated() {
    let mut v = separable(120, 4, 5);
    // payee of category 1 (merchantb), but the rule says category 3
    v.push(tx(900, "2026-10-04", "Merchantb Store 1", 21.0, 1, None));
    let c = cache_of(v, 4);
    let i = c.txns.binary_search_by_key(&900, |t| t.id).unwrap();
    let norm = model_of(&c).norms[i].clone();
    let mut r = rule(1, &norm, 3);
    r.matched["amount_min"] = json!(20.0);
    r.matched["amount_max"] = json!(22.0);
    let off = RuleRow { enabled: false, ..rule(2, &norm, 4) };
    let m = Model::build(&c, &Aliases::none(), &[off, r.clone()]);
    assert!(m.suggest(&c).enabled);
    let o = suggest::options(&m, &c, i, true);
    assert_eq!((o[0].source, o[0].category_id, o[0].confidence), ("rule", 3, 1.0));
    assert!(o[1..].iter().all(|x| x.source == "model" && x.category_id != 3));
    assert!(o.len() >= 2 && o[1].category_id == 1, "{o:?}");

    // outside the amount band the rule does not fire
    let c2 = cache_of(vec![tx(902, "2026-10-04", "Merchantb Store 1", 90.0, 1, None)], 4);
    let m2 = Model::build(&c2, &Aliases::none(), &[r]);
    assert!(suggest::options(&m2, &c2, 0, true).is_empty());
    // a rule option survives the disabled gate; payee_norm_any also matches
    let mut any = rule(3, "nobody", 2);
    any.matched = json!({ "payee_norm_any": ["nobody", norm], "amount_max": 100.0 });
    let m3 = Model::build(&c2, &Aliases::none(), &[any]);
    let o3 = suggest::options(&m3, &c2, 0, true);
    assert_eq!((o3.len(), o3[0].category_id), (1, 2));
}

#[test]
fn suggest_perf_50k_rows() {
    let mut s = 3u64;
    let mut v = Vec::with_capacity(50_000);
    for i in 0..50_000i64 {
        let cat = 1 + (lcg(&mut s) % 40) as i64;
        let payee = format!("Merchant{}{} Store {}", (b'a' + (cat / 6) as u8) as char, (b'a' + (cat % 6) as u8) as char, lcg(&mut s) % 7);
        let amount = cat as f64 * 3.0 + (lcg(&mut s) % 1000) as f64 / 100.0;
        let acct = 1 + (lcg(&mut s) % 3) as i64;
        let day = 1 + lcg(&mut s) % 28;
        let month = 1 + lcg(&mut s) % 11;
        v.push(tx(i + 1, &format!("2026-{month:02}-{day:02}"), &payee, amount, acct, if i % 20 == 0 { None } else { Some(cat) }));
    }
    let c = cache_of(v, 40);
    let norms: Vec<String> = c.txns.iter().map(|t| crate::smart::normalize::normalize(&t.payee, &Aliases::none())).collect();
    let t0 = std::time::Instant::now();
    let s = suggest::Suggest::train(&c, &norms);
    let ms = t0.elapsed().as_millis();
    eprintln!("train+CV on 50k rows: {ms} ms (top1 {:.3} top3 {:.3})", s.accuracy.top1, s.accuracy.top3);
    assert!(s.enabled && s.accuracy.top3 > 0.9, "{:?}", s.accuracy);
    let budget = if cfg!(debug_assertions) { 20_000 } else { 50 };
    assert!(ms < budget, "train+CV took {ms} ms (budget {budget})");
}

// ---- review -----------------------------------------------------------------------------------

#[test]
fn review_frequency_mode_keeps_payee_groups_together() {
    let v = vec![
        tx(1, "2026-10-01", "Zeta Mart", 5.0, 1, None),
        tx(2, "2026-10-09", "Alpha Cafe", 6.0, 1, None),
        tx(3, "2026-10-03", "Zeta Mart", 7.0, 1, None),
        tx(4, "2026-10-05", "Solo Shop", 8.0, 1, None),
        tx(5, "2026-10-02", "Alpha Cafe", 9.0, 1, None),
        tx(6, "2026-10-04", "Zeta Mart", 3.0, 1, None),
        tx(7, "2026-10-01", "Labeled", 3.0, 1, Some(1)),
    ];
    let c = cache_of(v, 1);
    let m = model_of(&c);
    let q = review::queue(&c, &m, 50, 0);
    assert!(!q.suggest_mode);
    assert_eq!(q.remaining, 6);
    let ids: Vec<i64> = q.rows.iter().map(|r| c.txns[r.idx].id).collect();
    // zeta (3 rows) newest first, alpha (2), solo (1)
    assert_eq!(ids, vec![6, 3, 1, 2, 5, 4]);
    assert_eq!(q.rows.iter().map(|r| r.payee_count).collect::<Vec<_>>(), vec![3, 3, 3, 2, 2, 1]);
    assert!(q.rows.iter().all(|r| r.options.is_empty()));
    let page = review::queue(&c, &m, 2, 3);
    assert_eq!(page.rows.iter().map(|r| c.txns[r.idx].id).collect::<Vec<_>>(), vec![2, 5]);
    assert_eq!(page.remaining, 6);
    // a matching rule still shows its option in bootstrap mode
    let norm = m.norms[c.txns.binary_search_by_key(&4, |t| t.id).unwrap()].clone();
    let m2 = Model::build(&c, &Aliases::none(), &[rule(1, &norm, 1)]);
    let q2 = review::queue(&c, &m2, 50, 0);
    let solo = q2.rows.iter().find(|r| c.txns[r.idx].id == 4).unwrap();
    assert_eq!(solo.options.len(), 1);
    assert_eq!(review::report(&c, &q2)["mode"], "frequency");
}

#[test]
fn review_suggest_mode_sorts_by_confidence_then_newest() {
    let mut v = separable(160, 4, 21);
    v.push(tx(2000, "2026-10-01", "Merchantb Store 1", 21.0, 1, None));
    v.push(tx(2001, "2026-10-09", "Merchantc Store 1", 41.0, 1, None));
    v.push(tx(2002, "2026-10-05", "Totally Unknown Place", 5.5, 1, None));
    let c = cache_of(v, 4);
    let m = model_of(&c);
    let q = review::queue(&c, &m, 50, 0);
    assert!(q.suggest_mode);
    assert_eq!((q.remaining, q.rows.len()), (3, 3));
    for w in q.rows.windows(2) {
        let (a, b) = (w[0].options.first().map_or(0.0, |o| o.confidence), w[1].options.first().map_or(0.0, |o| o.confidence));
        assert!(a >= b, "sorted by confidence");
    }
    let j = review::report(&c, &q);
    assert_eq!(j["mode"], "suggest");
    assert_eq!(j["rows"][0]["options"][0]["source"], "model");
}
