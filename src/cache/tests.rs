use super::*;
use crate::db::{RawAccount, RawCategory, RawTxn, Snapshot};

fn cat(id: i64, parent_id: Option<i64>, name: &str, sort: i64) -> RawCategory {
    RawCategory { id, parent_id, name: name.into(), kind: "expense".into(), sort }
}

fn txn(id: i64, amount: f64, cat: Option<i64>) -> RawTxn {
    RawTxn { id, date: "2026-01-02".into(), payee: Some("P".into()), amount, category_local_id: cat, ..Default::default() }
}

fn tree_snapshot() -> Snapshot {
    Snapshot {
        categories: vec![
            cat(1, None, "Home", 1),
            cat(2, Some(1), "Improvement", 1),
            cat(3, Some(2), "Paint", 0),
            cat(4, Some(1), "Garden", 0),
            cat(5, None, "Salary", 2),
            cat(6, Some(5), "Job", 0),
            cat(7, Some(1), "Zeta", 0),
        ],
        ..Default::default()
    }
}

#[test]
fn tree_is_in_display_order_with_paths_and_sizes() {
    let c = Cache::build(tree_snapshot());
    let names: Vec<&str> = c.categories.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(
        names,
        vec!["Home", "Home › Garden", "Home › Zeta", "Home › Improvement", "Home › Improvement › Paint", "Salary", "Salary › Job"]
    );
    let depth: Vec<u32> = c.categories.iter().map(|n| n.depth).collect();
    assert_eq!(depth, vec![0, 1, 1, 1, 2, 0, 1]);
    let size: Vec<u32> = c.categories.iter().map(|n| n.size).collect();
    assert_eq!(size, vec![5, 1, 1, 2, 1, 2, 1]);
    assert_eq!(c.node_pos(3), Some(4));
    assert_eq!(c.node_pos(404), None);
}

#[test]
fn orphans_and_cycles_appear_once_at_the_top() {
    let snap = Snapshot {
        categories: vec![
            cat(1, Some(99), "Orphan", 0), // parent missing
            cat(2, Some(3), "A", 0),       // 2 <-> 3 cycle
            cat(3, Some(2), "B", 0),
            cat(4, Some(4), "Self", 0), // self-parent
        ],
        ..Default::default()
    };
    let c = Cache::build(snap);
    let mut ids: Vec<i64> = c.categories.iter().map(|n| n.id).collect();
    assert_eq!(ids.len(), 4);
    ids.sort();
    assert_eq!(ids, vec![1, 2, 3, 4]);
    // every subtree range stays inside the vector
    for (i, n) in c.categories.iter().enumerate() {
        assert!(i + n.size as usize <= c.categories.len(), "{}", n.name);
    }
}

#[test]
fn counts_and_sums_roll_up_the_tree() {
    let mut snap = tree_snapshot();
    snap.txns = vec![
        txn(1, 10.0, Some(3)),  // Paint
        txn(2, -5.0, Some(3)),  // Paint, money in
        txn(3, 7.5, Some(4)),   // Garden
        txn(4, 100.0, Some(6)), // Job
        txn(5, 2.0, None),      // uncategorized
        txn(6, 1.0, Some(77)),  // unknown local id: treated as uncategorized
    ];
    let c = Cache::build(snap);
    let get = |name: &str| c.categories.iter().find(|n| n.name == name).unwrap();
    assert_eq!((get("Paint").count, get("Paint").count_subtree, get("Paint").sum_out_subtree), (2, 2, 10.0));
    assert_eq!((get("Improvement").count, get("Improvement").count_subtree), (0, 2));
    assert_eq!((get("Home").count, get("Home").count_subtree), (0, 3));
    assert_eq!(get("Home").sum_out_subtree, 17.5); // Paint 10 + Garden 7.5, money in ignored
    assert_eq!((get("Salary").count_subtree, get("Job").count), (1, 1));
    assert_eq!(c.uncategorized, 2);
    assert_eq!(c.txns[5].category_id, None);
    assert_eq!(c.txns[5].cat_pos, None);
    assert_eq!(c.txns[0].category_id, Some(3));
    assert_eq!(c.category_path(&c.txns[0]), Some("Home › Improvement › Paint"));
}

#[test]
fn accounts_use_display_names_balances_and_totals() {
    let snap = Snapshot {
        accounts: vec![
            RawAccount { id: 1, name: "zeta raw".into(), display_name: None, kind: Some("credit".into()), institution: Some("Bank".into()), mask: Some("1234".into()), currency: Some("USD".into()) },
            RawAccount { id: 2, name: "alpha raw".into(), display_name: Some("Bravo".into()), ..Default::default() },
            RawAccount { id: 3, name: "unused".into(), display_name: Some("".into()), ..Default::default() },
        ],
        txns: vec![
            RawTxn { account_id: Some(1), account: Some("zeta raw".into()), ..txn(1, 4.0, None) },
            RawTxn { account_id: Some(1), account: Some("zeta raw".into()), ..txn(2, -9.0, None) },
            RawTxn { account_id: Some(2), account: Some("Bravo".into()), ..txn(3, 1.25, None) },
        ],
        balances: vec![(2, 500.0)],
        ..Default::default()
    };
    let c = Cache::build(snap);
    let names: Vec<&str> = c.accounts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["Bravo", "unused", "zeta raw"]);
    let zeta = c.accounts.iter().find(|a| a.id == 1).unwrap();
    assert_eq!((zeta.count, zeta.sum_out, zeta.balance), (2, 4.0, None));
    assert_eq!((zeta.kind.as_deref(), zeta.mask.as_deref()), (Some("credit"), Some("1234")));
    let bravo = c.accounts.iter().find(|a| a.id == 2).unwrap();
    assert_eq!((bravo.count, bravo.balance), (1, Some(500.0)));
    let unused = c.accounts.iter().find(|a| a.id == 3).unwrap();
    assert_eq!((unused.count, unused.sum_out), (0, 0.0));
}

#[test]
fn transaction_fields_are_normalised() {
    let snap = Snapshot {
        txns: vec![
            RawTxn { id: 1, date: "2026-02-30".into(), payee: None, currency: None, ..Default::default() },
            RawTxn { id: 2, date: "2026-02-01".into(), tag_ids: Some("[1,2]".into()), ..Default::default() },
            RawTxn { id: 3, date: "2026-02-01".into(), tag_ids: Some("{\"x\":1}".into()), ..Default::default() },
            RawTxn { id: 4, date: "2026-02-01".into(), tag_ids: Some("not json".into()), ..Default::default() },
            RawTxn { id: 5, date: "2026-02-01".into(), status: Some("recurring_suggested".into()), ..Default::default() },
            RawTxn { id: 6, date: "2026-02-01".into(), recurring_id: Some(9), ..Default::default() },
            RawTxn { id: 7, date: "2026-02-01".into(), status: Some("cleared".into()), ..Default::default() },
        ],
        ..Default::default()
    };
    let c = Cache::build(snap);
    assert_eq!(c.txns[0].date_key, 0);
    assert_eq!((c.txns[0].payee.as_str(), c.txns[0].currency.as_str()), ("", ""));
    assert_eq!(c.txns[1].tags, serde_json::json!([1, 2]));
    assert_eq!(c.txns[2].tags, serde_json::json!([]));
    assert_eq!(c.txns[3].tags, serde_json::json!([]));
    assert_eq!(c.txns[1].date_key, 20260201);
    // a status string alone does not make a row recurring; the recurring_id does
    assert!(!c.txns[4].recurring);
    assert!(c.txns[5].recurring);
    assert!(!c.txns[6].recurring);
}

#[test]
fn payee_ranks_ignore_case() {
    let snap = Snapshot {
        txns: vec![
            RawTxn { id: 1, date: "2026-01-01".into(), payee: Some("Shell".into()), amount: 1.0, ..Default::default() },
            RawTxn { id: 2, date: "2026-01-01".into(), payee: Some("SHELL".into()), amount: 1.0, ..Default::default() },
            RawTxn { id: 3, date: "2026-01-01".into(), payee: Some("aardvark".into()), amount: 1.0, ..Default::default() },
        ],
        ..Default::default()
    };
    let c = Cache::build(snap);
    assert_eq!(c.txns[0].key_payee, c.txns[1].key_payee);
    assert!(c.txns[2].key_payee < c.txns[0].key_payee);
}

#[test]
fn synthetic_build_is_consistent() {
    let c = Cache::build(crate::cache::testing::synthetic(2_000));
    let categorized: usize = c.txns.iter().filter(|t| t.cat_pos.is_some()).count();
    assert_eq!(categorized + c.uncategorized, c.txns.len());
    // the root totals equal the sum over all categorized rows
    let root_total: usize = c.categories.iter().filter(|n| n.depth == 0).map(|n| n.count_subtree).sum();
    assert_eq!(root_total, categorized);
    assert_eq!(c.accounts.iter().map(|a| a.count).sum::<usize>(), c.txns.iter().filter(|t| t.account_id.is_some()).count());
}

/// The real loader: a synthetic SQLCipher file, migrated, then loaded into the cache.
#[test]
fn load_from_database_resolves_names_and_tree() {
    let (dir, path) = crate::db::tests::setup();
    let db = crate::db::Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    let c = Cache::load(&db).unwrap();
    assert_eq!(c.txns.len(), 4);
    assert_eq!(c.uncategorized, 4);
    let paths: Vec<&str> = c.categories.iter().map(|n| n.path.as_str()).collect();
    assert!(paths.contains(&"Home › Improvement › Paint"), "{:?}", paths);
    assert!(paths.contains(&"Salary › Job"), "{:?}", paths);
    let names: Vec<&str> = c.accounts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["Checking", "Manual Cash"]);
    let t2 = c.txns.iter().find(|t| t.id == 2).unwrap();
    assert_eq!(t2.account.as_deref(), Some("Manual Cash"));
    assert_eq!(t2.lm_category.as_deref(), Some("LM Fun"));
    let t3 = c.txns.iter().find(|t| t.id == 3).unwrap();
    assert_eq!((t3.payee.as_str(), t3.account.as_deref()), ("", None));
    let _ = std::fs::remove_dir_all(&dir);
}
