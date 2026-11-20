use super::*;
use rusqlite::Connection;
use std::os::unix::fs::PermissionsExt;

pub(crate) const PASS: &str = "test-pass-synthetic";

pub(crate) fn setup() -> (std::path::PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("omafin-test-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')', ' '], ""));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("finances.db");
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "key", PASS).unwrap();
    c.execute_batch(include_str!("../../schema.sql")).unwrap();
    c.execute_batch(include_str!("../../migration_multilevel_categories.sql")).unwrap();
    c.execute_batch(
        "INSERT INTO accounts(id,name,display_name) VALUES (1,'Chk raw','Checking'),(2,'Manual Cash',NULL);
         INSERT INTO categories(id,name) VALUES (10,'LM Groceries'),(11,'LM Fun');
         INSERT INTO category_main(id,name,type,display_order) VALUES (1,'Home','Expense',1),(2,'Salary','Income',2),(3,'Old','Expense',3);
         UPDATE category_main SET is_active=0 WHERE id=3;
         INSERT INTO category_sub(id,main_id,name,display_order) VALUES (1,1,'Improvement',1),(2,2,'Job',1),(3,3,'Hidden',1);
         INSERT INTO category_subsub(id,sub_id,name,display_order) VALUES (1,1,'Paint',1);
         INSERT INTO transactions(id,date,payee,amount,currency,category_id,plaid_account_id) VALUES
           (1,'2026-01-01','A',1.5,'USD',10,1),
           (2,'2026-01-03','B',2.5,NULL,11,NULL),
           (3,'2026-01-03',NULL,3.5,'USD',NULL,NULL),
           (4,'2025-12-31','D',4.5,'USD',10,1);
         UPDATE transactions SET manual_account_id=2 WHERE id=2;
         INSERT INTO sync_log(started_at,finished_at,status) VALUES
           ('a','2026-01-02 10:00','success'),('b','2026-01-05 10:00','error'),('c','2026-01-04 10:00','success');",
    )
    .unwrap();
    drop(c);
    (dir, path)
}

fn count(db: &Db, sql: &str) -> i64 {
    db.conn.lock().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn open_errors() {
    let (dir, path) = setup();
    let e = Db::open(&path, "nope-wrong").err().expect("wrong passphrase must fail").to_string();
    assert!(e.contains("wrong passphrase") && !e.contains("nope-wrong"));
    assert!(Db::open(&dir.join("missing.db"), PASS).is_err());
    assert!(!dir.join("missing.db").exists());
    assert!(Db::open(&path, PASS).is_ok());
}

#[test]
fn migrate_backup_seed_status_txns() {
    let (dir, path) = setup();
    let db = Db::open(&path, PASS).unwrap();
    assert_eq!(db.status().unwrap().schema_version, 0);
    assert_eq!(db.status().unwrap().categories, 0);

    let rep = db.migrate().unwrap();
    assert_eq!((rep.from, rep.to), (0, 1));
    let b = rep.backup.expect("backup");
    assert_eq!(std::fs::metadata(&b).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(b.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(b.file_name().unwrap().to_str().unwrap().ends_with("-v0.db"));
    // backup stays encrypted: opens with key, not without
    assert!(Db::open(&b, "bad").is_err());
    assert!(Db::open(&b, PASS).is_ok());
    assert_eq!(count(&db, "PRAGMA user_version"), 1);

    // idempotent
    let n_cat = count(&db, "SELECT count(*) FROM category");
    let rep2 = db.migrate().unwrap();
    assert_eq!((rep2.from, rep2.to), (1, 1));
    assert!(rep2.backup.is_none());
    assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 1);
    assert_eq!(count(&db, "SELECT count(*) FROM category"), n_cat);

    // seed: Home > Improvement > Paint, Salary > Job; inactive skipped
    assert_eq!(n_cat, 5);
    assert_eq!(count(&db, "SELECT count(*) FROM category WHERE name IN ('Old','Hidden')"), 0);
    let paint_path: String = db.conn.lock().unwrap().query_row(
        "SELECT g.name||'/'||p.name||'/'||c.name FROM category c JOIN category p ON c.parent_id=p.id JOIN category g ON p.parent_id=g.id WHERE c.legacy_table='category_subsub'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(paint_path, "Home/Improvement/Paint");
    assert_eq!(count(&db, "SELECT count(*) FROM category WHERE parent_id IS NULL"), 2);
    let kind: String = db.conn.lock().unwrap().query_row("SELECT kind FROM category WHERE name='Job'", [], |r| r.get(0)).unwrap();
    assert_eq!(kind, "income");
    assert_eq!(count(&db, "SELECT count(*) FROM sqlite_master WHERE type='trigger'"), 0);
    assert_eq!(count(&db, "SELECT count(*) FROM sqlite_master WHERE sql LIKE '%VIRTUAL%'"), 0);
    assert_eq!(count(&db, "SELECT count(*) FROM category_main"), 3); // legacy untouched

    // status
    let st = db.status().unwrap();
    assert_eq!((st.schema_version, st.transactions, st.accounts, st.categories), (1, 4, 2, 5));
    assert_eq!(st.last_txn_date.as_deref(), Some("2026-01-03"));
    assert_eq!(st.last_sync.as_deref(), Some("2026-01-04 10:00"));

    // transactions: local path vs LM fallback
    db.conn.lock().unwrap().execute(
        "UPDATE transactions SET category_local_id=(SELECT id FROM category WHERE name='Paint') WHERE id=1", []).unwrap();
    let (total, rows) = db.transactions(10, 0).unwrap();
    assert_eq!(total, 4);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3, 2, 1, 4]);
    assert_eq!(rows[0].payee, "");
    assert_eq!(rows[0].category, None);
    assert_eq!(rows[0].account, None);
    assert_eq!(rows[1].currency, "");
    assert_eq!(rows[1].account.as_deref(), Some("Manual Cash"));
    assert_eq!(rows[1].category.as_deref(), Some("LM Fun"));
    assert_eq!(rows[2].category.as_deref(), Some("Home › Improvement › Paint"));
    assert_eq!(rows[2].account.as_deref(), Some("Checking"));
    assert_eq!(rows[3].category.as_deref(), Some("LM Groceries"));

    // paging + clamps
    let (_, p) = db.transactions(2, 1).unwrap();
    assert_eq!(p.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(db.transactions(0, 0).unwrap().1.len(), 1);
    assert_eq!(db.transactions(-5, -3).unwrap().1.len(), 1);
    assert_eq!(db.transactions(10_000, 0).unwrap().1.len(), 4);
    assert!(db.transactions(5, 100).unwrap().1.is_empty());

    // sync-style upsert leaves category_local_id alone
    db.conn.lock().unwrap().execute(
        "INSERT INTO transactions(id,date,payee,amount) VALUES (1,'2026-01-01','Renamed',1.5)
         ON CONFLICT(id) DO UPDATE SET payee=excluded.payee", []).unwrap();
    assert_eq!(count(&db, "SELECT category_local_id IS NOT NULL FROM transactions WHERE id=1"), 1);
    let (_, rows) = db.transactions(10, 0).unwrap();
    assert_eq!(rows[2].payee, "Renamed");
    assert_eq!(rows[2].category.as_deref(), Some("Home › Improvement › Paint"));
    let _ = std::fs::remove_dir_all(&dir);
}
