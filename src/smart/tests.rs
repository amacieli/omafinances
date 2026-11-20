//! Preview golden tests on synthetic encrypted databases (the shared `fresh()` fixture), through the real `Cache`.

use super::*;
use super::normalize::AliasRow;
use crate::cache::Cache;
use crate::db::categories::tests::{create, fresh, raw_exec};
use crate::db::Db;
use std::collections::BTreeSet;
use std::time::Instant;

/// (id, date, payee, amount, category_local_id, recurring_id). Amounts: positive = money out.
type Row<'a> = (i64, &'a str, &'a str, f64, Option<i64>, Option<i64>);

fn insert(db: &Db, rows: &[Row]) {
    for chunk in rows.chunks(400) {
        let vals: Vec<String> = chunk
            .iter()
            .map(|(id, date, payee, amount, cat, rec)| {
                let cat = cat.map_or("NULL".to_string(), |c| c.to_string());
                let rec = rec.map_or("NULL".to_string(), |r| r.to_string());
                format!("({id},'{date}','{}',{amount},'USD',{cat},{rec},1)", payee.replace('\'', "''"))
            })
            .collect();
        raw_exec(
            db,
            &format!(
                "INSERT INTO transactions(id,date,payee,amount,currency,category_local_id,recurring_id,plaid_account_id) VALUES {}",
                vals.join(",")
            ),
        );
    }
}

fn req(ids: &[i64], category_id: i64) -> Request {
    Request { ids: ids.to_vec(), category_id, threshold: DEFAULT_THRESHOLD, overwrite: false }
}

fn cluster_with<'a>(p: &'a Preview, id: i64) -> &'a ClusterOut {
    p.clusters.iter().find(|c| c.txn_ids.contains(&id)).unwrap_or_else(|| panic!("txn {id} not in any cluster"))
}

fn run(db: &Db, r: &Request) -> Preview {
    let cache = Cache::load(db).unwrap();
    preview(&cache, &Aliases::none(), r).unwrap()
}

#[test]
fn golden_amazon_and_square_payees_group_exactly() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert(
        &db,
        &[
            (1001, "2026-01-05", "AMZN Mktp US*1A2B3", 42.10, None, None),
            (1002, "2026-02-05", "AMAZON MKTP US*9Z8Y7", 19.99, None, None),
            (1003, "2026-03-05", "Amazon.com*2K3L4", 5.00, None, None),
            (1004, "2026-01-07", "SQ *COFFEE SHOP #12 PORTLAND OR", 4.50, None, None),
            (1005, "2026-02-07", "COFFEE SHOP #98 SEATTLE WA", 4.75, None, None),
            (1006, "2026-03-09", "SQ *COFFEE SHOP #77 PORTLAND OR", 4.50, None, None),
        ],
    );
    let p = run(&db, &req(&[1001, 1004], groceries));

    let all: BTreeSet<i64> = p.clusters.iter().flat_map(|c| c.txn_ids.iter().copied()).collect();
    assert_eq!(all, [1001, 1002, 1003, 1004, 1005, 1006].into_iter().collect());
    let total_listed: usize = p.clusters.iter().map(|c| c.txn_ids.len()).sum();
    assert_eq!(total_listed, all.len(), "a txn appears in at most one cluster");

    let amazon = cluster_with(&p, 1002);
    assert_eq!(cluster_with(&p, 1003).id, amazon.id);
    assert_eq!(amazon.label, "amazon");
    assert_eq!(amazon.tier, Tier::Exact);
    assert_eq!(amazon.txn_ids, vec![1003, 1002], "sorted by date desc");
    assert!(amazon.default_selected);

    let coffee = cluster_with(&p, 1005);
    assert_eq!(coffee.label, "coffee shop");
    assert_eq!(cluster_with(&p, 1006).id, coffee.id);

    let seed = cluster_with(&p, 1001);
    assert_eq!(seed.tier, Tier::Seed);
    assert_eq!(seed.confidence, 1.0);
    assert_eq!(p.clusters[0].tier, Tier::Seed, "seed clusters come first");
    assert_eq!(cluster_with(&p, 1004).tier, Tier::Seed);

    assert_eq!(p.summary.matched, 6);
    assert_eq!(p.summary.selected, 6);
    assert_eq!(p.summary.selected_total, 80.84);
    assert_eq!(p.summary.conflicts, 0);
    assert_eq!(p.category.path, "Groceries");
}

#[test]
fn conflicts_same_category_and_overwrite() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    let shopping = create(&db, None, "Shopping");
    insert(
        &db,
        &[
            (2001, "2026-01-02", "Shell Oil", 50.0, None, None),
            (2002, "2026-01-09", "SHELL OIL 57442 PORTLAND OR", 40.0, Some(shopping), None),
            (2003, "2026-01-16", "SHELL OIL #9", 30.0, Some(groceries), None),
        ],
    );
    let p = run(&db, &req(&[2001], groceries));
    let conflict = cluster_with(&p, 2002);
    assert_eq!(conflict.status, Status::Conflict);
    assert_eq!(conflict.conflict_categories, vec!["Shopping".to_string()]);
    assert!(!conflict.default_selected, "conflicts are off unless overwrite");
    let same = cluster_with(&p, 2003);
    assert_eq!(same.status, Status::Same);
    assert!(!same.default_selected);
    assert!(same.conflict_categories.is_empty());
    assert_eq!(p.summary.conflicts, 1);
    assert_eq!(p.summary.same, 1);
    assert_eq!(p.summary.selected, 1, "only the uncategorized seed is selected");

    let mut r = req(&[2001], groceries);
    r.overwrite = true;
    let p = run(&db, &r);
    assert!(cluster_with(&p, 2002).default_selected, "conflicts selected with overwrite");
    assert!(!cluster_with(&p, 2003).default_selected, "same-category stays unselected");
}

#[test]
fn fuzzy_recurring_amount_band_and_sign_tiers() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert(
        &db,
        &[
            (3001, "2026-01-03", "STARBUCKS COFFEE", 6.25, None, None),
            (3002, "2026-01-04", "STARBUCKS COFFE 0042", 5.75, None, None),
            (3010, "2026-01-05", "Gym Club", 30.0, None, Some(77)),
            (3011, "2026-02-05", "FITCLUB INC", 30.0, None, Some(77)),
            (3020, "2026-01-06", "ACME PAYROLL", -2500.0, None, None),
            (3021, "2026-01-07", "ACME PAYROLL", -2500.0, None, None),
            (3022, "2026-01-08", "ACME PAYROLL", 2500.0, None, None),
            (3030, "2026-01-09", "Blue Bottle Coffee", 12.0, None, None),
            (3031, "2026-01-10", "Blue Ridge Tavern", 12.5, None, None),
            (3032, "2026-01-11", "Blue Ridge Tavern", 25.0, None, None),
        ],
    );
    let seeds = [3001, 3010, 3020, 3030];
    let p = run(&db, &req(&seeds, groceries));

    let fuzzy = cluster_with(&p, 3002);
    assert_eq!(fuzzy.tier, Tier::Fuzzy);
    assert!(fuzzy.confidence >= DEFAULT_THRESHOLD && fuzzy.confidence < 1.0, "{}", fuzzy.confidence);
    assert!(fuzzy.reason.starts_with("payee ") && fuzzy.reason.contains("similar to 'starbucks coffee'"), "{}", fuzzy.reason);

    let rec = cluster_with(&p, 3011);
    assert_eq!(rec.tier, Tier::Recurring);
    assert_eq!(rec.confidence, 0.9);
    assert_eq!(rec.reason, "same recurring id");

    assert_eq!(cluster_with(&p, 3021).tier, Tier::Exact);
    assert!(
        p.clusters.iter().all(|c| !c.txn_ids.contains(&3022)),
        "income seed must never match an expense row"
    );

    let band = cluster_with(&p, 3031);
    assert_eq!(band.tier, Tier::AmountBand);
    assert!(band.confidence >= 0.75 && band.confidence <= 0.8, "{}", band.confidence);
    assert!(p.clusters.iter().all(|c| !c.txn_ids.contains(&3032)), "25 is outside the 25% band of 12");

    // a higher threshold drops the amount-band candidate (its confidence is below 0.8)
    let mut strict = req(&seeds, groceries);
    strict.threshold = 0.8;
    let p = run(&db, &strict);
    assert!(p.clusters.iter().all(|c| !c.txn_ids.contains(&3031)));
    assert!(p.clusters.iter().all(|c| c.confidence >= 0.8 || c.tier == Tier::Seed));
    // the recurring and exact matches survive it
    assert_eq!(cluster_with(&p, 3011).tier, Tier::Recurring);
}

#[test]
fn exemplars_are_distinct_and_cover_the_cluster_shape() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert(
        &db,
        &[
            (4001, "2026-01-10", "AMZN MKTP", 10.0, None, None),
            (4002, "2026-01-01", "AMAZON", 10.0, None, None),
            (4003, "2026-06-01", "AMAZON", 50.0, None, None),
            (4004, "2026-03-01", "AMAZON", 12.0, None, None),
            (4005, "2026-04-01", "AMAZON", 11.0, None, None),
        ],
    );
    let p = run(&db, &req(&[4001], groceries));
    let c = cluster_with(&p, 4002);
    assert_eq!(c.count, 4);
    let ex: Vec<(i64, &str)> = c.exemplars.iter().map(|e| (e.id, e.why)).collect();
    // earliest 4002, latest 4003, median (lower middle of 10,11,12,50) 4005; the outlier 4003 is already listed
    assert_eq!(ex, vec![(4002, "earliest"), (4003, "latest"), (4005, "median")]);
    let mut ids: Vec<i64> = c.exemplars.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), c.exemplars.len());
    assert_eq!(c.txn_ids, vec![4003, 4005, 4004, 4002], "sorted by date desc");
    assert_eq!((c.date_min.as_str(), c.date_max.as_str()), ("2026-01-01", "2026-06-01"));
    assert_eq!((c.amount_min, c.amount_max, c.total), (10.0, 50.0, 83.0));
}

#[test]
fn preview_is_deterministic_and_validates_input() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert(
        &db,
        &[
            (5001, "2026-01-05", "AMZN MKTP", 10.0, None, None),
            (5002, "2026-01-06", "AMAZON", 11.0, None, None),
        ],
    );
    let cache = Cache::load(&db).unwrap();
    let a = preview(&cache, &Aliases::none(), &req(&[5001], groceries)).unwrap();
    let b = preview(&cache, &Aliases::none(), &req(&[5001], groceries)).unwrap();
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());

    let none = Aliases::none();
    for t in [0.4, 1.01, f64::NAN] {
        let mut r = req(&[5001], groceries);
        r.threshold = t;
        assert!(matches!(preview(&cache, &none, &r), Err(SmartError::Invalid(_))), "threshold {t}");
    }
    assert!(matches!(preview(&cache, &none, &req(&[5001], 999_999)), Err(SmartError::NotFound(_))));
    assert!(matches!(preview(&cache, &none, &req(&[999_999], groceries)), Err(SmartError::Invalid(_))));
    assert!(matches!(preview(&cache, &none, &req(&[], groceries)), Err(SmartError::Invalid(_))));
    let too_many: Vec<i64> = (1..=501).collect();
    assert!(matches!(preview(&cache, &none, &req(&too_many, groceries)), Err(SmartError::Invalid(_))));
    // unknown ids are ignored when at least one exists
    let p = preview(&cache, &none, &req(&[999_999, 5001], groceries)).unwrap();
    assert_eq!(p.summary.selected, 2);
}

#[test]
fn aliases_change_the_grouping_key() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert(
        &db,
        &[
            (6001, "2026-01-05", "AMZN Mktp US*1A2B3", 10.0, None, None),
            (6002, "2026-01-06", "Marketplace order 77", 30.0, None, None),
        ],
    );
    let cache = Cache::load(&db).unwrap();
    let aliases = Aliases::compile(&[AliasRow {
        pattern: "^marketplace order".into(),
        canonical: "Amazon Marketplace".into(),
        is_regex: true,
    }]);
    let p = preview(&cache, &aliases, &req(&[6001], groceries)).unwrap();
    // the alias collapses "AMZN Mktp" to "amazon", and the regex row to "amazon marketplace", so no exact match
    assert_eq!(p.clusters.len(), 1, "no other rows match");
    let p = preview(&cache, &aliases, &req(&[6002], groceries)).unwrap();
    assert_eq!(p.clusters[0].label, "amazon marketplace");
}

/// 50k synthetic rows over ~1500 distinct normalized payees, with income rows and noisy suffixes. The seeds are the
/// 33 rows of one payee. Only the preview is timed; loading the cache is not part of the request budget.
#[test]
fn fifty_thousand_rows_preview_is_fast() {
    const A: [&str; 30] = [
        "coffee", "shell", "netflix", "amazon", "costco", "target", "walmart", "chipotle", "uber", "lyft", "spotify",
        "hardware", "pharmacy", "grocery", "bakery", "cinema", "gym", "florist", "pizza", "bookshop", "dental", "vet",
        "garden", "tailor", "laundry", "parking", "transit", "tavern", "dry clean", "cafe",
    ];
    const B: [&str; 50] = [
        "north", "south", "east", "west", "main", "oak", "pine", "lake", "river", "hill", "park", "bay", "valley",
        "ridge", "mill", "elm", "cedar", "birch", "maple", "harbor", "station", "plaza", "market", "corner", "central",
        "union", "summit", "meadow", "grove", "bridge", "port", "gate", "forest", "canyon", "delta", "crest", "point",
        "field", "stone", "brook", "orchard", "glen", "haven", "terrace", "square", "quay", "row", "ward", "yard", "mesa",
    ];
    let (_d, db) = fresh();
    let target = create(&db, None, "PerfCat");
    let n = 50_000i64;
    let mut rows: Vec<Row> = Vec::with_capacity(n as usize);
    let payees: Vec<String> = (0..n)
        .map(|i| {
            let k = i % 1500;
            format!("SQ *{} {} #{} PORTLAND OR", A[(k % 30) as usize], B[(k / 30) as usize], i)
        })
        .collect();
    let dates: Vec<String> = (0..n).map(|i| format!("2025-{:02}-{:02}", 1 + i % 12, 1 + i % 28)).collect();
    for i in 0..n {
        let amount = if i % 50 == 0 { -(2000.0 + (i % 7) as f64) } else { 5.0 + (i % 97) as f64 };
        rows.push((100_000 + i, &dates[i as usize], &payees[i as usize], amount, None, None));
    }
    insert(&db, &rows);

    let cache = Cache::load(&db).unwrap();
    let seeds: Vec<i64> = (0..n).filter(|i| i % 1500 == 0).map(|i| 100_000 + i).collect();
    assert_eq!(seeds.len(), 34);
    let r = req(&seeds, target);
    let aliases = Aliases::none();

    let started = Instant::now();
    let p = preview(&cache, &aliases, &r).unwrap();
    let elapsed = started.elapsed();
    eprintln!("preview over {n} rows: {:?} ({} clusters, {} matched)", elapsed, p.clusters.len(), p.summary.matched);
    assert!(p.summary.matched >= seeds.len());
    let budget = if cfg!(debug_assertions) { 2_000 } else { 50 };
    assert!(
        elapsed.as_millis() < budget,
        "preview took {:?}, budget {budget} ms ({} build)",
        elapsed,
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
}
