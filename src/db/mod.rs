//! Encrypted (SQLCipher) database access. One `Mutex<Connection>`: key derivation is slow, so we
//! open once. Journal mode is left alone (the file lives on NFS; no WAL).
pub mod migrations;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub struct Db {
    pub path: PathBuf,
    #[allow(dead_code)]
    conn: Mutex<Connection>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Status {
    pub schema_version: i64,
    pub transactions: i64,
    pub accounts: i64,
    pub categories: i64,
    pub last_txn_date: Option<String>,
    pub last_sync: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TxnRow {
    pub id: i64,
    pub date: String,
    pub payee: String,
    pub amount: f64,
    pub currency: String,
    pub account: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MigrateReport {
    pub from: i64,
    pub to: i64,
    pub backup: Option<PathBuf>,
}

impl Db {
    pub fn open(path: &Path, passphrase: &str) -> Result<Db> {
        if !path.is_file() {
            bail!("database file not found: {}", path.display());
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| anyhow!("cannot open database {}: {}", path.display(), e))?;
        conn.pragma_update(None, "key", passphrase).map_err(|_| anyhow!("failed to apply database key"))?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .map_err(|_| anyhow!("cannot read database: wrong passphrase or not a database"))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Db { path: path.to_path_buf(), conn: Mutex::new(conn) })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| anyhow!("database lock poisoned"))
    }

    pub fn migrate(&self) -> Result<MigrateReport> {
        let mut conn = self.lock()?;
        let from: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let pending: Vec<_> = migrations::MIGRATIONS.iter().filter(|m| m.0 > from).collect();
        if pending.is_empty() {
            return Ok(MigrateReport { from, to: from, backup: None });
        }
        let backup = self.backup(&conn, from)?;
        let mut to = from;
        for (ver, desc, f) in pending {
            let tx = conn.transaction()?;
            f(&tx).with_context(|| format!("migration v{ver} ({desc}) failed"))?;
            tx.pragma_update(None, "user_version", *ver)?;
            tx.commit()?;
            to = *ver;
        }
        Ok(MigrateReport { from, to, backup: Some(backup) })
    }

    /// Copy the (still encrypted) db file into `<dir>/backups/` before migrating.
    fn backup(&self, conn: &Connection, from: i64) -> Result<PathBuf> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let _ = conn; // we hold the connection lock, so nothing of ours writes during the copy
        let dir = self.path.parent().unwrap_or_else(|| Path::new(".")).join("backups");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let dest = dir.join(format!("finances-{stamp}-v{from}.db"));
        let mut src = std::fs::File::open(&self.path)?;
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&dest)
            .with_context(|| format!("creating backup {}", dest.display()))?;
        std::io::copy(&mut src, &mut out)?;
        out.sync_all()?;
        Ok(dest)
    }

    pub fn status(&self) -> Result<Status> {
        let conn = self.lock()?;
        let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let count = |t: &str| -> Result<i64> {
            if !migrations::table_exists(&conn, t)? {
                return Ok(0);
            }
            Ok(conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))?)
        };
        let last_txn_date = conn.query_row("SELECT max(date) FROM transactions", [], |r| r.get::<_, Option<String>>(0))?;
        let last_sync = if migrations::table_exists(&conn, "sync_log")? {
            conn.query_row("SELECT max(finished_at) FROM sync_log WHERE status='success'", [], |r| r.get::<_, Option<String>>(0))?
        } else {
            None
        };
        Ok(Status {
            schema_version,
            transactions: count("transactions")?,
            accounts: count("accounts")?,
            categories: count("category")?,
            last_txn_date,
            last_sync,
        })
    }

    pub fn transactions(&self, limit: i64, offset: i64) -> Result<(i64, Vec<TxnRow>)> {
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let conn = self.lock()?;
        let has_local = migrations::column_exists(&conn, "transactions", "category_local_id")?;
        let total: i64 = conn.query_row("SELECT count(*) FROM transactions", [], |r| r.get(0))?;

        let mut tree: HashMap<i64, (Option<i64>, String)> = HashMap::new();
        if has_local && migrations::table_exists(&conn, "category")? {
            let mut s = conn.prepare("SELECT id,parent_id,name FROM category")?;
            for row in s.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, String>(2)?)))? {
                let (id, p, n) = row?;
                tree.insert(id, (p, n));
            }
        }
        let local_expr = if has_local { "t.category_local_id" } else { "NULL" };
        let sql = format!(
            "SELECT t.id, t.date, COALESCE(t.payee,''), t.amount, COALESCE(t.currency,''),
                    COALESCE(NULLIF(a.display_name,''), a.name, NULLIF(m.display_name,''), m.name),
                    {local_expr}, c.name
             FROM transactions t
             LEFT JOIN accounts a ON a.id = t.plaid_account_id
             LEFT JOIN accounts m ON m.id = t.manual_account_id
             LEFT JOIN categories c ON c.id = t.category_id
             ORDER BY t.date DESC, t.id DESC LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([limit, offset], |r| {
            Ok((
                TxnRow {
                    id: r.get(0)?,
                    date: r.get(1)?,
                    payee: r.get(2)?,
                    amount: r.get(3)?,
                    currency: r.get(4)?,
                    account: r.get(5)?,
                    category: None,
                },
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (mut t, local, lm) = row?;
            t.category = match local {
                Some(id) if tree.contains_key(&id) => Some(path_of(&tree, id)),
                _ => lm,
            };
            out.push(t);
        }
        Ok((total, out))
    }
}

fn path_of(tree: &HashMap<i64, (Option<i64>, String)>, id: i64) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(id);
    while let Some(i) = cur {
        match tree.get(&i) {
            Some((p, n)) if parts.len() < 16 => {
                parts.push(n.clone());
                cur = *p;
            }
            _ => break,
        }
    }
    parts.reverse();
    parts.join(" › ")
}

#[cfg(test)]
mod tests;
