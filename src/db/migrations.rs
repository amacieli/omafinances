//! Ordered schema migrations, tracked with `PRAGMA user_version`.
//!
//! Rules: no triggers and no FTS tables (the Python sync writes the same file through a
//! sqlcipher3 build that may lack FTS5). Legacy tables are never modified.
use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction};

pub type MigrationFn = fn(&Transaction) -> Result<()>;

/// (target version, description, apply fn). Append only.
pub const MIGRATIONS: &[(i64, &str, MigrationFn)] = &[
    (1, "category tree, journal, rules, views", v1),
    (2, "undo/redo: superseded flag on change_batch", v2),
    (3, "reports: saved reports and run log", v3),
];

pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|m| m.0).unwrap_or(0)
}

pub fn table_exists(c: &Connection, name: &str) -> Result<bool> {
    Ok(c.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1", [name], |r| r.get::<_, i64>(0))? > 0)
}

pub fn column_exists(c: &Connection, table: &str, col: &str) -> Result<bool> {
    Ok(c.query_row("SELECT count(*) FROM pragma_table_info(?1) WHERE name=?2", params![table, col], |r| r.get::<_, i64>(0))? > 0)
}

fn v1(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS category (
            id INTEGER PRIMARY KEY,
            parent_id INTEGER REFERENCES category(id) ON DELETE RESTRICT,
            name TEXT NOT NULL,
            kind TEXT NOT NULL DEFAULT 'expense' CHECK (kind IN ('expense','income','transfer')),
            sort INTEGER DEFAULT 0,
            color TEXT,
            icon TEXT,
            legacy_table TEXT,
            legacy_id INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS ux_category_parent_name ON category(COALESCE(parent_id,0), name COLLATE NOCASE);
        CREATE TABLE IF NOT EXISTS change_batch (
            id INTEGER PRIMARY KEY,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            label TEXT,
            kind TEXT,
            undone_at TEXT
        );
        CREATE TABLE IF NOT EXISTS change_item (
            id INTEGER PRIMARY KEY,
            batch_id INTEGER NOT NULL REFERENCES change_batch(id),
            entity TEXT NOT NULL,
            entity_id INTEGER,
            field TEXT,
            old_value TEXT,
            new_value TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_change_item_batch ON change_item(batch_id);
        CREATE TABLE IF NOT EXISTS rule (
            id INTEGER PRIMARY KEY,
            position INTEGER NOT NULL DEFAULT 0,
            name TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            match_json TEXT NOT NULL DEFAULT '{}',
            category_id INTEGER REFERENCES category(id),
            hits INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS saved_view (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            query TEXT,
            sort TEXT,
            columns TEXT,
            position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS payee_alias (
            id INTEGER PRIMARY KEY,
            pattern TEXT NOT NULL,
            canonical TEXT NOT NULL,
            is_regex INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    if !column_exists(tx, "transactions", "category_local_id")? {
        tx.execute_batch("ALTER TABLE transactions ADD COLUMN category_local_id INTEGER REFERENCES category(id);")?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_transactions_category_local ON transactions(category_local_id);")?;
    seed_categories(tx).context("seeding category tree from legacy tables")?;
    Ok(())
}

/// Redo support: a batch created after an undo supersedes the undone batches, so they can no longer be redone.
fn v2(tx: &Transaction) -> Result<()> {
    if !column_exists(tx, "change_batch", "superseded")? {
        tx.execute_batch("ALTER TABLE change_batch ADD COLUMN superseded INTEGER NOT NULL DEFAULT 0;")?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_change_batch_undo ON change_batch(undone_at, superseded);")?;
    Ok(())
}

/// Scheduled reports and their run log. `trigger` is quoted because it is a SQL keyword.
fn v3(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS report (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            query TEXT NOT NULL DEFAULT '',
            shape TEXT NOT NULL,
            period TEXT NOT NULL,
            schedule_json TEXT NOT NULL,
            recipients_json TEXT NOT NULL,
            redact INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS report_run (
            id INTEGER PRIMARY KEY,
            report_id INTEGER NOT NULL REFERENCES report(id) ON DELETE CASCADE,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            status TEXT NOT NULL,
            detail TEXT,
            rows INTEGER,
            message_id TEXT,
            \"trigger\" TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_report_run_report ON report_run(report_id, id);",
    )?;
    Ok(())
}

fn kind_of(t: Option<String>) -> String {
    match t.as_deref().map(|s| s.to_lowercase()) {
        Some(k) if k == "income" || k == "transfer" || k == "expense" => k,
        _ => "expense".to_string(),
    }
}

/// Insert a node; on a (case-insensitive) sibling name clash reuse the existing row.
fn insert_node(tx: &Transaction, parent: Option<i64>, name: &str, kind: &str, sort: i64, table: &str, legacy_id: i64) -> Result<i64> {
    let n = tx.execute(
        "INSERT OR IGNORE INTO category(parent_id,name,kind,sort,legacy_table,legacy_id) VALUES (?1,?2,?3,?4,?5,?6)",
        params![parent, name, kind, sort, table, legacy_id],
    )?;
    if n > 0 {
        return Ok(tx.last_insert_rowid());
    }
    Ok(tx.query_row(
        "SELECT id FROM category WHERE COALESCE(parent_id,0)=COALESCE(?1,0) AND name=?2 COLLATE NOCASE",
        params![parent, name],
        |r| r.get(0),
    )?)
}

fn seed_categories(tx: &Transaction) -> Result<()> {
    if !(table_exists(tx, "category_main")? && table_exists(tx, "category_sub")? && table_exists(tx, "category_subsub")?) {
        return Ok(());
    }
    if tx.query_row("SELECT count(*) FROM category", [], |r| r.get::<_, i64>(0))? > 0 {
        return Ok(());
    }
    let mains: Vec<(i64, String, Option<String>, i64)> = {
        let mut s = tx.prepare("SELECT id,name,type,COALESCE(display_order,0) FROM category_main WHERE is_active<>0 ORDER BY display_order,id")?;
        let v = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        v
    };
    for (mid, mname, mtype, msort) in mains {
        let kind = kind_of(mtype);
        let main_new = insert_node(tx, None, &mname, &kind, msort, "category_main", mid)?;
        let subs: Vec<(i64, String, i64)> = {
            let mut s = tx.prepare("SELECT id,name,COALESCE(display_order,0) FROM category_sub WHERE main_id=?1 AND is_active<>0 ORDER BY display_order,id")?;
            let v = s.query_map([mid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            v
        };
        for (sid, sname, ssort) in subs {
            let sub_new = insert_node(tx, Some(main_new), &sname, &kind, ssort, "category_sub", sid)?;
            let subsubs: Vec<(i64, String, i64)> = {
                let mut s = tx.prepare("SELECT id,name,COALESCE(display_order,0) FROM category_subsub WHERE sub_id=?1 AND is_active<>0 ORDER BY display_order,id")?;
                let v = s.query_map([sid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
                v
            };
            for (ssid, ssname, sssort) in subsubs {
                insert_node(tx, Some(sub_new), &ssname, &kind, sssort, "category_subsub", ssid)?;
            }
        }
    }
    Ok(())
}
