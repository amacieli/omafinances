//! Undo/redo tests: every mutation kind must undo to a byte-equal state and redo to the state after it.
use super::*;
use crate::db::categories::tests::{create, exists, fresh, names, raw_exec, row, snap, sorts, txn_cat};
use crate::db::migrations;

/// apply -> state B; undo -> state A exactly; redo -> state B exactly.
fn roundtrip(db: &Db, act: impl FnOnce(&Db)) {
    let before = snap(db);
    act(db);
    let after = snap(db);
    assert_ne!(before, after, "the action must change something for the test to mean anything");
    db.undo().expect("undo");
    assert_eq!(snap(db), before, "undo restores the exact prior state");
    db.redo().expect("redo");
    assert_eq!(snap(db), after, "redo reproduces the state after the change");
}

#[test]
fn undo_redo_create() {
    let (_d, db) = fresh();
    let p = create(&db, None, "UrCreateP");
    roundtrip(&db, |db| {
        create(db, Some(p), "UrNew");
    });
}

#[test]
fn undo_redo_rename_and_kind() {
    let (_d, db) = fresh();
    let p = create(&db, None, "UrRenP");
    let c = create(&db, Some(p), "UrRenC");
    roundtrip(&db, |db| {
        db.rename_category(c, "Renamed").unwrap();
    });
    roundtrip(&db, |db| {
        db.set_category_kind(p, "income").unwrap();
    });
}

#[test]
fn undo_redo_move_restores_sort_values() {
    let (_d, db) = fresh();
    let p = create(&db, None, "UrMvP");
    create(&db, Some(p), "a");
    let b = create(&db, Some(p), "b");
    let c = create(&db, Some(p), "c");
    let q = create(&db, None, "UrMvQ");
    create(&db, Some(q), "q1");
    roundtrip(&db, |db| {
        db.move_category(3, c, Some(p), Some(0)).unwrap();
    });
    roundtrip(&db, |db| {
        db.move_category(3, b, Some(q), None).unwrap();
    });
    roundtrip(&db, |db| {
        db.move_category_within(3, c, 1).unwrap();
    });
}

#[test]
fn undo_redo_delete_with_reassign_and_promotion() {
    let (_d, db) = fresh();
    let p = create(&db, None, "UrDelP");
    let q = create(&db, Some(p), "UrDelQ");
    let q1 = create(&db, Some(q), "UrDelQ1");
    create(&db, Some(p), "UrDelSib");
    let target = create(&db, None, "UrDelTarget");
    db.categorize(&[1, 2], Some(q)).unwrap();
    db.categorize(&[3], Some(q1)).unwrap();
    roundtrip(&db, |db| {
        db.delete_category(q, Some(target)).unwrap();
    });
}

#[test]
fn undo_redo_delete_uncategorize() {
    let (_d, db) = fresh();
    let q = create(&db, None, "UrDelNoneQ");
    db.categorize(&[1], Some(q)).unwrap();
    roundtrip(&db, |db| {
        db.delete_category(q, None).unwrap();
    });
}

#[test]
fn undo_redo_merge_with_recursive_merge() {
    let (_d, db) = fresh();
    let s = create(&db, None, "UrMgSrc");
    let sc = create(&db, Some(s), "Common");
    create(&db, Some(sc), "X");
    create(&db, Some(s), "Solo");
    let d = create(&db, None, "UrMgDst");
    create(&db, Some(d), "common");
    db.categorize(&[1], Some(s)).unwrap();
    db.categorize(&[2], Some(sc)).unwrap();
    roundtrip(&db, |db| {
        db.merge_category(3, s, d).unwrap();
    });
}

#[test]
fn undo_redo_categorize_and_uncategorize() {
    let (_d, db) = fresh();
    let q = create(&db, None, "UrCzQ");
    roundtrip(&db, |db| {
        db.categorize(&[1, 2, 3], Some(q)).unwrap();
    });
    roundtrip(&db, |db| {
        db.categorize(&[1, 4], None).unwrap();
    });
}

#[test]
fn undo_redo_saved_views_save_update_and_delete() {
    let (_d, db) = fresh();
    roundtrip(&db, |db| {
        db.view_save("UrView", "p:a", Some("date:desc")).unwrap();
    });
    let v = db.view_save("UrKeep", "p:b", None).unwrap();
    roundtrip(&db, |db| {
        db.view_save("UrKeep", "p:c", None).unwrap();
    });
    roundtrip(&db, |db| {
        assert!(db.view_delete(v.id).unwrap());
    });
}

#[test]
fn undo_and_redo_walk_a_whole_sequence() {
    let (_d, db) = fresh();
    let start = snap(&db);
    let p = create(&db, None, "SeqP");
    let c = create(&db, Some(p), "SeqC");
    db.categorize(&[1], Some(c)).unwrap();
    db.rename_category(c, "SeqRenamed").unwrap();
    let end = snap(&db);
    for _ in 0..4 {
        db.undo().unwrap();
    }
    assert_eq!(snap(&db), start);
    assert!(matches!(db.undo(), Err(MutError::NothingToUndo)));
    for _ in 0..4 {
        db.redo().unwrap();
    }
    assert_eq!(snap(&db), end);
    assert!(matches!(db.redo(), Err(MutError::NothingToRedo)));
}

#[test]
fn redo_is_invalidated_by_a_new_change() {
    let (_d, db) = fresh();
    create(&db, None, "SupA");
    db.undo().unwrap();
    let j = db.journal(10).unwrap();
    assert!(j.can_redo && j.next_redo_label.as_deref() == Some("Create \"SupA\""));
    let b = create(&db, None, "SupB");
    assert!(matches!(db.redo(), Err(MutError::NothingToRedo)), "creating a batch supersedes undone ones");
    let j = db.journal(10).unwrap();
    let rows: Vec<(String, bool, bool)> = j.rows.iter().map(|r| (r.label.clone(), r.undone, r.superseded)).collect();
    assert_eq!(
        rows,
        vec![
            ("Create \"SupB\"".to_string(), false, false),
            ("Create \"SupA\"".to_string(), true, true),
        ]
    );
    assert!(!j.can_redo && j.next_redo_label.is_none());
    // SQLite reuses the freed rowid, so judge by name, not by id.
    let top = names(&db, None);
    assert!(top.iter().any(|n| n == "SupB") && !top.iter().any(|n| n == "SupA"));
    assert!(exists(&db, b));
}

#[test]
fn redo_order_is_oldest_undone_first() {
    let (_d, db) = fresh();
    let a = create(&db, None, "OrdA");
    let b = create(&db, None, "OrdB");
    db.undo().unwrap(); // B
    db.undo().unwrap(); // A
    assert!(!exists(&db, a) && !exists(&db, b));
    let first = db.redo().unwrap();
    assert_eq!(first.label, "Create \"OrdA\"", "the batch undone last comes back first");
    let second = db.redo().unwrap();
    assert_eq!(second.label, "Create \"OrdB\"");
    assert!(exists(&db, a) && exists(&db, b));
}

#[test]
fn undo_refuses_when_a_category_changed_underneath() {
    let (_d, db) = fresh();
    let x = create(&db, None, "CfRename");
    db.rename_category(x, "CfRenamed").unwrap();
    raw_exec(&db, &format!("UPDATE category SET name='CfElsewhere' WHERE id={x}"));
    let before = snap(&db);
    match db.undo() {
        Err(MutError::Conflict(m)) => assert!(m.contains("changed"), "{m}"),
        other => panic!("expected conflict, got {other:?}"),
    }
    assert_eq!(snap(&db), before, "a refused undo changes nothing");
    assert!(db.journal(5).unwrap().can_undo, "the batch stays on the undo stack");
}

#[test]
fn undo_refuses_when_a_transaction_changed_underneath() {
    let (_d, db) = fresh();
    let q = create(&db, None, "CfTxn");
    db.categorize(&[1], Some(q)).unwrap();
    raw_exec(&db, "UPDATE transactions SET category_local_id=NULL WHERE id=1");
    let before = snap(&db);
    assert!(matches!(db.undo(), Err(MutError::Conflict(_))));
    assert_eq!(snap(&db), before);
    assert_eq!(txn_cat(&db, 1), None);
}

#[test]
fn undo_refuses_when_a_view_changed_underneath() {
    let (_d, db) = fresh();
    db.view_save("CfView", "p:a", None).unwrap();
    raw_exec(&db, "UPDATE saved_view SET query='tampered' WHERE name='CfView'");
    assert!(matches!(db.undo(), Err(MutError::Conflict(_))));
}

#[test]
fn nothing_to_undo_or_redo_on_an_empty_journal() {
    let (_d, db) = fresh();
    assert!(matches!(db.undo(), Err(MutError::NothingToUndo)));
    assert!(matches!(db.redo(), Err(MutError::NothingToRedo)));
    let j = db.journal(20).unwrap();
    assert!(!j.can_undo && !j.can_redo && j.next_undo_label.is_none());
}

#[test]
fn journal_lists_newest_first_with_limit_and_next_labels() {
    let (_d, db) = fresh();
    create(&db, None, "JlOne");
    create(&db, None, "JlTwo");
    let p = create(&db, None, "JlThree");
    db.categorize(&[1], Some(p)).unwrap();
    let j = db.journal(2).unwrap();
    assert_eq!(j.rows.len(), 2);
    assert_eq!(j.rows[0].label, "Categorize 1 transaction as JlThree");
    assert_eq!(j.rows[0].kind, "categorize");
    assert_eq!(j.rows[1].kind, "category");
    assert_eq!(j.next_undo_label.as_deref(), Some("Categorize 1 transaction as JlThree"));
    assert!(!j.can_redo);
    db.undo().unwrap();
    let j = db.journal(100).unwrap();
    assert!(j.rows[0].undone && !j.rows[0].superseded);
    assert_eq!(j.next_redo_label.as_deref(), Some("Categorize 1 transaction as JlThree"));
    assert_eq!(j.next_undo_label.as_deref(), Some("Create \"JlThree\""));
    // limit is clamped on the db side as well
    assert_eq!(db.journal(0).unwrap().rows.len(), 1);
}

#[test]
fn undoing_a_delete_restores_child_order_and_sorts() {
    let (_d, db) = fresh();
    let p = create(&db, None, "OrdP");
    let q = create(&db, Some(p), "OrdQ");
    let k1 = create(&db, Some(q), "k1");
    let k2 = create(&db, Some(q), "k2");
    create(&db, Some(p), "after");
    let before = (names(&db, Some(p)), sorts(&db, Some(p)), names(&db, Some(q)), sorts(&db, Some(q)));
    db.delete_category(q, None).unwrap();
    db.undo().unwrap();
    assert_eq!((names(&db, Some(p)), sorts(&db, Some(p)), names(&db, Some(q)), sorts(&db, Some(q))), before);
    assert_eq!(row(&db, k1).parent_id, Some(q));
    assert_eq!(row(&db, k2).parent_id, Some(q));
}

/// The migration may be run again on a database that already has the column and index.
#[test]
fn migration_v2_is_rerunnable() {
    let (_d, db) = fresh();
    let f = migrations::MIGRATIONS.iter().find(|m| m.0 == 2).expect("v2 registered").2;
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        f(&tx).unwrap();
        f(&tx).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(migrations::latest_version(), 2);
    let conn = db.conn.lock().unwrap();
    assert!(migrations::column_exists(&conn, "change_batch", "superseded").unwrap());
}

/// A v1 database (no `superseded` column) migrates once, keeps its history, and writes one backup.
#[test]
fn migrating_from_v1_keeps_history_and_writes_one_backup() {
    let (dir, db) = fresh();
    create(&db, None, "MvOne");
    db.undo().unwrap();
    let before = db.journal(100).unwrap().rows.len();
    {
        let c = db.conn.lock().unwrap();
        c.execute_batch("DROP INDEX idx_change_batch_undo; ALTER TABLE change_batch DROP COLUMN superseded; PRAGMA user_version=1;")
            .unwrap();
    }
    let backups = || std::fs::read_dir(dir.join("backups")).unwrap().count();
    let first = backups();
    let rep = db.migrate().unwrap();
    assert_eq!((rep.from, rep.to), (1, 2));
    assert!(rep.backup.is_some());
    assert_eq!(backups(), first + 1);
    assert_eq!(db.journal(100).unwrap().rows.len(), before, "history survives the migration");
    assert!(db.journal(100).unwrap().rows.iter().all(|r| !r.superseded));
    let again = db.migrate().unwrap();
    assert!(again.backup.is_none(), "nothing pending: no second backup");
    assert_eq!(backups(), first + 1);
}
