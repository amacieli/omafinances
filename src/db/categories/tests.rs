//! Category tree and categorization tests. Every test runs on a migrated synthetic encrypted database
//! (`crate::db::tests::setup`). Undo/redo tests live in `journal/tests.rs`.
use super::*;
use crate::db::journal::MutError;
use crate::db::tests::{setup, PASS};
use std::path::PathBuf;

pub(crate) fn fresh() -> (PathBuf, Db) {
    let (dir, path) = setup();
    let db = Db::open(&path, PASS).unwrap();
    db.migrate().unwrap();
    (dir, db)
}

/// Everything an undo must restore exactly: categories (all columns), transaction assignments, rules and
/// saved views. Row text is compared as SQLite values, so a changed sort or timestamp shows up.
pub(crate) fn snap(db: &Db) -> String {
    let c = db.conn.lock().unwrap();
    let queries = [
        "SELECT id,parent_id,name,kind,sort,color,icon,legacy_table,legacy_id,created_at,updated_at FROM category ORDER BY id",
        "SELECT id,category_local_id FROM transactions ORDER BY id",
        "SELECT id,position,name,enabled,match_json,category_id,hits,created_at FROM rule ORDER BY id",
        "SELECT id,name,query,sort,position FROM saved_view ORDER BY id",
    ];
    let mut out = String::new();
    for q in queries {
        let mut s = c.prepare(q).unwrap();
        let n = s.column_count();
        let rows = s
            .query_map([], |r| {
                let mut parts = Vec::new();
                for i in 0..n {
                    parts.push(format!("{:?}", r.get::<_, rusqlite::types::Value>(i)?));
                }
                Ok(parts.join("|"))
            })
            .unwrap();
        for r in rows {
            out.push_str(&r.unwrap());
            out.push('\n');
        }
        out.push_str("--\n");
    }
    out
}

pub(crate) fn create(db: &Db, parent: Option<i64>, name: &str) -> i64 {
    db.create_category(3, parent, name, None).expect(name).id.unwrap()
}

/// Sibling names in display order.
pub(crate) fn names(db: &Db, parent: Option<i64>) -> Vec<String> {
    let c = db.conn.lock().unwrap();
    children(&c, parent).unwrap().into_iter().map(|r| r.name).collect()
}

/// Sibling `sort` values in display order.
pub(crate) fn sorts(db: &Db, parent: Option<i64>) -> Vec<i64> {
    let c = db.conn.lock().unwrap();
    children(&c, parent).unwrap().into_iter().map(|r| r.sort.unwrap()).collect()
}

pub(crate) fn row(db: &Db, id: i64) -> CatRow {
    let c = db.conn.lock().unwrap();
    load(&c, id).unwrap().expect("category exists")
}

pub(crate) fn exists(db: &Db, id: i64) -> bool {
    let c = db.conn.lock().unwrap();
    load(&c, id).unwrap().is_some()
}

pub(crate) fn txn_cat(db: &Db, id: i64) -> Option<i64> {
    let c = db.conn.lock().unwrap();
    c.query_row("SELECT category_local_id FROM transactions WHERE id=?1", [id], |r| r.get(0)).unwrap()
}

pub(crate) fn raw_exec(db: &Db, sql: &str) {
    db.conn.lock().unwrap().execute(sql, []).unwrap();
}

#[test]
fn create_trims_validates_and_inherits_kind() {
    let (_d, db) = fresh();
    let top = db.create_category(3, None, "  Zeta  ", Some("income")).unwrap();
    let zeta = top.id.unwrap();
    assert_eq!(top.label, "Create \"Zeta\"");
    assert!(top.batch_id.is_some());
    assert_eq!(row(&db, zeta).name, "Zeta");

    let kid = db.create_category(3, Some(zeta), "kid", None).unwrap().id.unwrap();
    assert_eq!(row(&db, kid).kind, "income", "child inherits the parent's kind");
    let explicit = db.create_category(3, Some(zeta), "exp", Some("expense")).unwrap().id.unwrap();
    assert_eq!(row(&db, explicit).kind, "expense");
    let top_default = create(&db, None, "PlainTop");
    assert_eq!(row(&db, top_default).kind, "expense");

    assert!(matches!(db.create_category(3, Some(zeta), "KID", None), Err(MutError::Conflict(_))), "case-insensitive sibling clash");
    assert!(matches!(db.create_category(3, None, "   ", None), Err(MutError::Invalid(_))));
    assert!(matches!(db.create_category(3, None, &"x".repeat(81), None), Err(MutError::Invalid(_))));
    assert!(db.create_category(3, None, &"y".repeat(80), None).is_ok(), "80 characters is allowed");
    assert!(matches!(db.create_category(3, None, "a\tb", None), Err(MutError::Invalid(_))));
    assert!(matches!(db.create_category(3, None, "ok", Some("bogus")), Err(MutError::Invalid(_))));
    assert!(matches!(db.create_category(3, Some(999_999), "x", None), Err(MutError::NotFound(_))));
}

#[test]
fn create_sort_is_max_sibling_plus_one() {
    let (_d, db) = fresh();
    let p = create(&db, None, "SortParent");
    let a = create(&db, Some(p), "a");
    let b = create(&db, Some(p), "b");
    let c = create(&db, Some(p), "c");
    assert_eq!((row(&db, a).sort, row(&db, b).sort, row(&db, c).sort), (Some(0), Some(1), Some(2)));
    assert_eq!(names(&db, Some(p)), vec!["a", "b", "c"]);
}

#[test]
fn depth_limit_counts_levels_from_one() {
    let (_d, db) = fresh();
    let l1 = create(&db, None, "D1");
    let l2 = create(&db, Some(l1), "D2");
    let l3 = create(&db, Some(l2), "D3");
    match db.create_category(3, Some(l3), "D4", None) {
        Err(MutError::Limit(m)) => assert!(m.contains('3'), "message names the limit: {m}"),
        other => panic!("expected limit, got {other:?}"),
    }
    // A smaller limit applies to the same tree.
    assert!(matches!(db.create_category(2, Some(l2), "D3b", None), Err(MutError::Limit(_))));
    assert!(db.create_category(2, Some(l1), "D2b", None).is_ok());
}

#[test]
fn rename_checks_clash_and_noop() {
    let (_d, db) = fresh();
    let p = create(&db, None, "RnParent");
    let a = create(&db, Some(p), "Alpha");
    create(&db, Some(p), "Beta");
    assert!(matches!(db.rename_category(a, "beta"), Err(MutError::Conflict(_))));
    let noop = db.rename_category(a, "Alpha").unwrap();
    assert_eq!(noop.batch_id, None, "same name writes no batch");
    let case = db.rename_category(a, "alpha").unwrap();
    assert_eq!(case.label, "Rename \"Alpha\" to \"alpha\"");
    assert_eq!(row(&db, a).name, "alpha");
    assert!(matches!(db.rename_category(999_999, "x"), Err(MutError::NotFound(_))));
    assert!(matches!(db.rename_category(a, ""), Err(MutError::Invalid(_))));
}

#[test]
fn set_kind_changes_only_the_node() {
    let (_d, db) = fresh();
    let p = create(&db, None, "KindP");
    let c = create(&db, Some(p), "KindC");
    let out = db.set_category_kind(p, "income").unwrap();
    assert_eq!(out.label, "Set kind of \"KindP\" to income");
    assert_eq!(row(&db, p).kind, "income");
    assert_eq!(row(&db, c).kind, "expense", "children keep their kind");
    assert_eq!(db.set_category_kind(p, "income").unwrap().batch_id, None);
    assert!(matches!(db.set_category_kind(p, "savings"), Err(MutError::Invalid(_))));
}

#[test]
fn move_reparents_reorders_and_keeps_sorts_dense() {
    let (_d, db) = fresh();
    let p = create(&db, None, "MvP");
    let c1 = create(&db, Some(p), "c1");
    let c2 = create(&db, Some(p), "c2");
    let c3 = create(&db, Some(p), "c3");
    db.move_category(3, c3, Some(p), Some(0)).unwrap();
    assert_eq!(names(&db, Some(p)), vec!["c3", "c1", "c2"]);
    assert_eq!(sorts(&db, Some(p)), vec![0, 1, 2]);
    db.move_category(3, c1, Some(p), None).unwrap();
    assert_eq!(names(&db, Some(p)), vec!["c3", "c2", "c1"]);

    // Re-parent under another node: source siblings close the gap, destination opens it.
    let q = create(&db, None, "MvQ");
    let q1 = create(&db, Some(q), "q1");
    let out = db.move_category(3, c2, Some(q), Some(0)).unwrap();
    assert_eq!(out.label, "Move \"c2\" under \"MvQ\"");
    assert_eq!(names(&db, Some(p)), vec!["c3", "c1"]);
    assert_eq!(sorts(&db, Some(p)), vec![0, 1]);
    assert_eq!(names(&db, Some(q)), vec!["c2", "q1"]);
    assert_eq!(sorts(&db, Some(q)), vec![0, 1]);
    assert_eq!(row(&db, c2).parent_id, Some(q));
    assert_eq!(row(&db, q1).sort, Some(1));

    // Back to the top level.
    let out = db.move_category(3, c2, None, None).unwrap();
    assert_eq!(out.label, "Move \"c2\" to the top level");
    assert_eq!(row(&db, c2).parent_id, None);
    assert_eq!(names(&db, Some(q)), vec!["q1"]);
}

#[test]
fn move_refuses_cycles_clashes_and_depth_overflow() {
    let (_d, db) = fresh();
    let p = create(&db, None, "CyP");
    let c = create(&db, Some(p), "CyC");
    let g = create(&db, Some(c), "CyG");
    let before = snap(&db);
    assert!(matches!(db.move_category(3, p, Some(g), None), Err(MutError::Conflict(_))), "under a descendant");
    assert!(matches!(db.move_category(3, p, Some(p), None), Err(MutError::Conflict(_))), "under itself");
    assert_eq!(snap(&db), before, "refused moves change nothing");

    // Sibling name clash at the destination.
    create(&db, Some(c), "dupe");
    let top_dupe = create(&db, None, "dupe");
    assert!(matches!(db.move_category(3, top_dupe, Some(c), None), Err(MutError::Conflict(_))));

    // Depth: P(1) > C(2) > G(3). A two-level subtree cannot land under C (its leaf would be 4).
    let x = create(&db, None, "XRoot");
    create(&db, Some(x), "XKid");
    match db.move_category(3, x, Some(c), None) {
        Err(MutError::Limit(m)) => assert!(m.contains('4'), "{m}"),
        other => panic!("expected limit, got {other:?}"),
    }
    assert!(matches!(db.move_category(3, x, Some(g), None), Err(MutError::Limit(_))));
    assert!(db.move_category(3, x, Some(p), None).is_ok(), "fits under level 1: leaf at level 3");
}

#[test]
fn move_within_swaps_with_neighbour_and_noops_at_edges() {
    let (_d, db) = fresh();
    let p = create(&db, None, "WiP");
    let a = create(&db, Some(p), "a");
    let b = create(&db, Some(p), "b");
    let c = create(&db, Some(p), "c");
    assert!(db.move_category_within(3, b, -1).unwrap().batch_id.is_some());
    assert_eq!(names(&db, Some(p)), vec!["b", "a", "c"]);
    assert_eq!(db.move_category_within(3, b, -1).unwrap().batch_id, None, "already first");
    db.move_category_within(3, b, 1).unwrap();
    assert_eq!(names(&db, Some(p)), vec!["a", "b", "c"]);
    assert_eq!(db.move_category_within(3, c, 1).unwrap().batch_id, None, "already last");
    assert!(matches!(db.move_category_within(3, a, 2), Err(MutError::Invalid(_))));
    assert_eq!(sorts(&db, Some(p)), vec![0, 1, 2]);
}

#[test]
fn delete_reassigns_own_transactions_and_promotes_children() {
    let (_d, db) = fresh();
    let p = create(&db, None, "DlP");
    let q = create(&db, Some(p), "DlQ");
    let q1 = create(&db, Some(q), "DlQ1");
    create(&db, Some(p), "DlSib");
    let target = create(&db, None, "DlTarget");
    db.categorize(&[1, 2], Some(q)).unwrap();
    db.categorize(&[3], Some(q1)).unwrap();

    // Reassign: own transactions go to the target; the grandchild's transaction stays.
    let out = db.delete_category(q, Some(target)).unwrap();
    assert_eq!(out.label, "Delete \"DlQ\" and move 2 transactions to \"DlTarget\"");
    assert!(!exists(&db, q));
    assert_eq!(txn_cat(&db, 1), Some(target));
    assert_eq!(txn_cat(&db, 2), Some(target));
    assert_eq!(txn_cat(&db, 3), Some(q1), "descendants keep their transactions");
    // Child promoted to the parent, appended after the existing siblings.
    assert_eq!(row(&db, q1).parent_id, Some(p));
    assert_eq!(names(&db, Some(p)), vec!["DlSib", "DlQ1"]);
    assert_eq!(sorts(&db, Some(p)), vec![0, 1]);
}

#[test]
fn delete_without_reassign_uncategorizes() {
    let (_d, db) = fresh();
    let q = create(&db, None, "DelNone");
    db.categorize(&[1], Some(q)).unwrap();
    let out = db.delete_category(q, None).unwrap();
    assert_eq!(out.label, "Delete \"DelNone\" and uncategorize 1 transaction");
    assert_eq!(txn_cat(&db, 1), None);
}

#[test]
fn delete_refuses_name_clash_and_bad_reassign_without_changes() {
    let (_d, db) = fresh();
    let p = create(&db, None, "DcP");
    let q = create(&db, Some(p), "DcQ");
    create(&db, Some(q), "same");
    create(&db, Some(p), "same");
    let q_kid = create(&db, Some(q), "inner");
    db.categorize(&[1], Some(q)).unwrap();
    let before = snap(&db);
    assert!(matches!(db.delete_category(q, None), Err(MutError::Conflict(_))), "child name clashes after promotion");
    assert!(matches!(db.delete_category(q, Some(q)), Err(MutError::Conflict(_))), "reassign to itself");
    assert!(matches!(db.delete_category(q, Some(q_kid)), Err(MutError::Conflict(_))), "reassign into its subtree");
    assert!(matches!(db.delete_category(999_999, None), Err(MutError::NotFound(_))));
    assert!(matches!(db.delete_category(p, Some(999_999)), Err(MutError::NotFound(_))));
    assert_eq!(snap(&db), before, "refused deletes change nothing");
}

#[test]
fn merge_moves_transactions_and_merges_same_named_children_recursively() {
    let (_d, db) = fresh();
    let s = create(&db, None, "MgSrc");
    let s_common = create(&db, Some(s), "Common");
    let s_x = create(&db, Some(s_common), "X");
    let s_solo = create(&db, Some(s), "Solo");
    let d = create(&db, None, "MgDst");
    let d_common = create(&db, Some(d), "common"); // same name, different case
    let d_y = create(&db, Some(d_common), "Y");
    db.categorize(&[1], Some(s)).unwrap();
    db.categorize(&[2], Some(s_common)).unwrap();
    db.categorize(&[3], Some(s_x)).unwrap();
    db.categorize(&[4], Some(s_solo)).unwrap();

    let out = db.merge_category(3, s, d).unwrap();
    assert_eq!(out.label, "Merge \"MgSrc\" into \"MgDst\"");
    assert!(!exists(&db, s), "source is deleted");
    assert!(!exists(&db, s_common), "same-named child merged, not kept");
    assert_eq!(txn_cat(&db, 1), Some(d));
    assert_eq!(txn_cat(&db, 2), Some(d_common), "transactions follow the merged child");
    assert_eq!(txn_cat(&db, 3), Some(s_x), "grandchild moved with its own id");
    assert_eq!(names(&db, Some(d)), vec!["common", "Solo"]);
    assert_eq!(row(&db, s_x).parent_id, Some(d_common));
    assert_eq!(names(&db, Some(d_common)), vec!["Y", "X"]);
    assert_eq!(row(&db, d_y).parent_id, Some(d_common));
    assert_eq!(sorts(&db, Some(d)), vec![0, 1]);
    assert_eq!(sorts(&db, Some(d_common)), vec![0, 1]);
}

#[test]
fn merge_refuses_self_and_descendant_targets() {
    let (_d, db) = fresh();
    let s = create(&db, None, "MrSelf");
    let kid = create(&db, Some(s), "MrKid");
    let before = snap(&db);
    assert!(matches!(db.merge_category(3, s, s), Err(MutError::Conflict(_))));
    assert!(matches!(db.merge_category(3, s, kid), Err(MutError::Conflict(_))));
    assert!(matches!(db.merge_category(3, 999_999, s), Err(MutError::NotFound(_))));
    assert_eq!(snap(&db), before);
}

#[test]
fn merge_refuses_depth_overflow_without_changes() {
    let (_d, db) = fresh();
    let s = create(&db, None, "MdSrc");
    let s1 = create(&db, Some(s), "s1");
    create(&db, Some(s1), "s2");
    let d = create(&db, None, "MdDst");
    let d1 = create(&db, Some(d), "d1");
    create(&db, Some(d1), "d2");
    db.categorize(&[1], Some(s)).unwrap();
    let before = snap(&db);
    // s1 would land at level 3 and its child at level 4 under d1 (level 2).
    assert!(matches!(db.merge_category(3, s, d1), Err(MutError::Limit(_))));
    assert_eq!(snap(&db), before);
    // With room for it, the same merge succeeds.
    assert!(db.merge_category(4, s, d1).is_ok());
    assert_eq!(txn_cat(&db, 1), Some(d1));
}

#[test]
fn categorize_counts_skips_and_labels() {
    let (_d, db) = fresh();
    let p = create(&db, None, "CzP");
    let q = create(&db, Some(p), "CzQ");
    let out = db.categorize(&[1, 2, 999_999, 1], Some(q)).unwrap();
    assert_eq!(out.changed, 2, "duplicates count once");
    assert_eq!(out.skipped, 1, "unknown id is skipped");
    assert_eq!(out.label, "Categorize 2 transactions as CzP › CzQ");
    assert!(out.batch_id.is_some());
    assert_eq!(txn_cat(&db, 1), Some(q));

    let again = db.categorize(&[1, 2], Some(q)).unwrap();
    assert_eq!((again.changed, again.skipped, again.batch_id), (0, 2, None), "already equal counts as skipped");

    let un = db.categorize(&[1], None).unwrap();
    assert_eq!((un.changed, un.label.as_str()), (1, "Uncategorize 1 transaction"));
    assert_eq!(txn_cat(&db, 1), None);
    assert_eq!(txn_cat(&db, 2), Some(q));
}

#[test]
fn categorize_validates_input() {
    let (_d, db) = fresh();
    assert!(matches!(db.categorize(&[], None), Err(MutError::Invalid(_))));
    let too_many: Vec<i64> = (0..=MAX_CATEGORY_IDS as i64).collect();
    assert!(matches!(db.categorize(&too_many, None), Err(MutError::Invalid(_))));
    let at_cap: Vec<i64> = vec![1; MAX_CATEGORY_IDS];
    assert!(db.categorize(&at_cap, None).is_ok(), "exactly the cap is allowed");
    assert!(matches!(db.categorize(&[1], Some(999_999)), Err(MutError::NotFound(_))));
}

#[test]
fn cache_reflects_writes_immediately_through_refresh() {
    let (_d, db) = fresh();
    let p = create(&db, None, "CcP");
    let c = create(&db, Some(p), "CcC");
    db.categorize(&[1], Some(c)).unwrap();
    let cache = crate::cache::Cache::load(&db).unwrap();
    let node = cache.categories.iter().find(|n| n.id == c).expect("new node in cache");
    // The cache's `depth` is 0-based (top level = 0), the same as the existing API contract.
    assert_eq!((node.depth, node.parent_id, node.sort), (1, Some(p), 0));
    assert_eq!(cache.txns.iter().find(|t| t.id == 1).unwrap().category_id, Some(c));
}
