//! Smart apply: one batch, undo/redo covers rows and rule, duplicate rules, overwrite=false, aliases.

use super::*;
use crate::db::categories::tests::{create, fresh, raw_exec, snap, txn_cat};
use serde_json::json;

fn insert_rows(db: &Db) {
    raw_exec(
        db,
        "INSERT INTO transactions(id,date,payee,amount,currency,category_local_id,plaid_account_id) VALUES
           (5001,'2026-01-05','AMZN Mktp US*1A2B3',40.0,'USD',NULL,1),
           (5002,'2026-01-06','AMAZON MKTP US*9Z8Y7',41.0,'USD',NULL,1),
           (5003,'2026-01-07','Amazon.com*5',42.0,'USD',NULL,1),
           (5004,'2026-01-08','Shell',10.0,'USD',NULL,1)",
    );
}

fn batch_kinds(db: &Db) -> Vec<String> {
    let c = db.conn.lock().unwrap();
    let mut s = c.prepare("SELECT kind FROM change_batch WHERE superseded=0 ORDER BY id").unwrap();
    s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
}

fn rule_count(db: &Db) -> i64 {
    let c = db.conn.lock().unwrap();
    c.query_row("SELECT count(*) FROM rule", [], |r| r.get(0)).unwrap()
}

#[test]
fn apply_writes_rows_and_one_smart_rule_batch() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert_rows(&db);
    let before = snap(&db);
    let a = db.smart_apply(groceries, &[5002, 5001], false, Some(None)).unwrap();

    assert_eq!(a.changed, 2);
    assert_eq!(a.skipped, 0);
    assert_eq!(a.skipped_conflicts, 0);
    assert!(a.batch_id.is_some());
    assert!(a.rule_id.is_some());
    assert_eq!(a.label, "Categorize 2 transactions as Groceries (+rule)");
    assert_eq!(txn_cat(&db, 5001), Some(groceries));
    assert_eq!(txn_cat(&db, 5002), Some(groceries));
    assert_eq!(txn_cat(&db, 5003), None, "rows not selected are untouched");
    assert_eq!(batch_kinds(&db).last().map(String::as_str), Some("smart"));

    let rules = db.list_rules().unwrap();
    assert_eq!(rules.len(), 1);
    let r = &rules[0];
    assert_eq!(r.name.as_deref(), Some("amazon"));
    assert!(r.enabled);
    assert_eq!(r.hits, 0);
    assert_eq!(r.category_id, Some(groceries));
    assert_eq!(r.category_path.as_deref(), Some("Groceries"));
    assert_eq!(r.matched, json!({"payee_norm": "amazon", "amount_min": 40.0, "amount_max": 41.0}));

    // one undo reverts the rows and the rule together; one redo brings both back
    let after = snap(&db);
    db.undo().unwrap();
    assert_eq!(snap(&db), before);
    assert_eq!(rule_count(&db), 0);
    db.redo().unwrap();
    assert_eq!(snap(&db), after);
    assert_eq!(rule_count(&db), 1);
}

#[test]
fn duplicate_rule_is_not_inserted_again() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert_rows(&db);
    let first = db.smart_apply(groceries, &[5001, 5002], false, Some(None)).unwrap();
    let rule = first.rule_id.unwrap();

    // the same rows again: nothing to write, the rule is the existing one
    let again = db.smart_apply(groceries, &[5001, 5002], false, Some(Some("Other name".into()))).unwrap();
    assert_eq!(again.changed, 0);
    assert_eq!(again.skipped, 2);
    assert_eq!(again.rule_id, Some(rule));
    assert_eq!(again.batch_id, None);
    assert_eq!(again.label, "No change");
    assert_eq!(rule_count(&db), 1);
}

#[test]
fn overwrite_false_never_changes_a_row_in_another_category() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    let shopping = create(&db, None, "Shopping");
    insert_rows(&db);
    db.smart_apply(shopping, &[5003], false, None).unwrap();

    let a = db.smart_apply(groceries, &[5001, 5003], false, None).unwrap();
    assert_eq!(a.changed, 1);
    assert_eq!(a.skipped_conflicts, 1);
    assert_eq!(txn_cat(&db, 5003), Some(shopping), "a different category is kept without overwrite");

    let b = db.smart_apply(groceries, &[5003], true, None).unwrap();
    assert_eq!(b.changed, 1);
    assert_eq!(b.skipped_conflicts, 0);
    assert_eq!(txn_cat(&db, 5003), Some(groceries), "overwrite moves it");
}

#[test]
fn unknown_ids_are_skipped_and_cannot_make_a_rule() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert_rows(&db);

    let a = db.smart_apply(groceries, &[999_999, 5004], false, None).unwrap();
    assert_eq!(a.changed, 1);
    assert_eq!(a.skipped, 1);

    assert!(matches!(db.smart_apply(groceries, &[888_888], false, Some(None)), Err(MutError::Invalid(_))));
    assert!(matches!(db.smart_apply(groceries, &[], false, None), Err(MutError::Invalid(_))));
    assert!(matches!(db.smart_apply(999_999, &[5004], false, None), Err(MutError::NotFound(_))));
    assert_eq!(rule_count(&db), 0);
}

#[test]
fn rule_name_is_trimmed_and_positions_grow() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert_rows(&db);
    let a = db.smart_apply(groceries, &[5004], false, Some(Some("  Shell gas  ".into()))).unwrap();
    assert!(a.rule_id.is_some());
    db.smart_apply(groceries, &[5001], false, Some(None)).unwrap();
    let rules = db.list_rules().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name.as_deref(), Some("Shell gas"));
    assert_eq!(rules[0].matched["payee_norm"], "shell");
    assert_eq!(rules[1].name.as_deref(), Some("amazon"));
    assert!(rules[0].position < rules[1].position);
}

#[test]
fn payee_aliases_decide_the_rule_key() {
    let (_d, db) = fresh();
    let groceries = create(&db, None, "Groceries");
    insert_rows(&db);
    raw_exec(&db, "INSERT INTO payee_alias(pattern, canonical, is_regex) VALUES ('amzn mktp us*1a2b3', 'Amazon Marketplace', 0)");
    db.smart_apply(groceries, &[5001, 5002], false, Some(None)).unwrap();
    let rules = db.list_rules().unwrap();
    assert_eq!(rules[0].matched["payee_norm_any"], json!(["amazon", "amazon marketplace"]));
}

#[test]
fn rules_listing_carries_the_category_path_and_no_match_is_an_error_free_empty_list() {
    let (_d, db) = fresh();
    assert!(db.list_rules().unwrap().is_empty());
    let groceries = create(&db, None, "Groceries");
    insert_rows(&db);
    db.smart_apply(groceries, &[5004], false, Some(None)).unwrap();
    assert_eq!(db.list_rules().unwrap()[0].category_path.as_deref(), Some("Groceries"));
}
