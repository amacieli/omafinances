//! Encrypted (SQLCipher) database access. One `Mutex<Connection>`: key derivation is slow, so we
//! open once. Journal mode is left alone (the file lives on NFS; no WAL).
pub mod accounts;
pub mod categories;
pub mod journal;
pub mod migrations;
pub mod notes;
pub mod reports;
pub mod rules;
pub mod views;

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
                    COALESCE(NULLIF(a.nickname,''), NULLIF(a.display_name,''), a.name, NULLIF(m.nickname,''), NULLIF(m.display_name,''), m.name),
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

/// One transaction as stored, with its account and Lunch Money category already resolved.
/// Nullable text stays `None`; the cache decides what to show.
#[derive(Debug, Clone, Default)]
pub struct RawTxn {
    pub id: i64,
    pub date: String,
    pub payee: Option<String>,
    pub original_name: Option<String>,
    pub notes: Option<String>,
    /// The user's own note (`user_notes`); `None` when unset or the column does not exist yet.
    pub user_notes: Option<String>,
    pub amount: f64,
    pub currency: Option<String>,
    pub status: Option<String>,
    pub source: Option<String>,
    pub tag_ids: Option<String>,
    pub recurring_id: Option<i64>,
    pub external_id: Option<String>,
    pub created_at_lm: Option<String>,
    pub updated_at_lm: Option<String>,
    pub is_pending: bool,
    pub category_id: Option<i64>,
    pub category_local_id: Option<i64>,
    pub lm_category: Option<String>,
    pub account_id: Option<i64>,
    pub account: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RawAccount {
    pub id: i64,
    pub name: String,
    pub display_name: Option<String>,
    /// User-editable nickname (migration v5); preferred over `display_name` for display.
    pub nickname: Option<String>,
    pub kind: Option<String>,
    pub institution: Option<String>,
    pub mask: Option<String>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RawCategory {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub sort: i64,
}

/// Everything the cache needs, read in one pass. `balances` holds the latest snapshot per account.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub txns: Vec<RawTxn>,
    pub accounts: Vec<RawAccount>,
    pub categories: Vec<RawCategory>,
    pub balances: Vec<(i64, f64)>,
}

impl Db {
    /// Read the whole mirror for the in-memory cache. Bound parameters only; no filtering here.
    pub fn load_all(&self) -> Result<Snapshot> {
        let conn = self.lock()?;
        if !migrations::column_exists(&conn, "transactions", "category_local_id")? {
            bail!("database is not migrated (no category_local_id column)");
        }
        if !migrations::column_exists(&conn, "transactions", "user_notes")? {
            bail!("database is not migrated (no user_notes column)");
        }
        if !migrations::column_exists(&conn, "accounts", "nickname")? {
            bail!("database is not migrated (no accounts.nickname column)");
        }
        let mut snap = Snapshot::default();

        if migrations::table_exists(&conn, "category")? {
            let mut s = conn.prepare("SELECT id, parent_id, name, COALESCE(kind,'expense'), COALESCE(sort,0) FROM category")?;
            snap.categories = s
                .query_map([], |r| {
                    Ok(RawCategory { id: r.get(0)?, parent_id: r.get(1)?, name: r.get(2)?, kind: r.get(3)?, sort: r.get(4)? })
                })?
                .collect::<rusqlite::Result<_>>()?;
        }

        let mut s = conn.prepare("SELECT id, name, display_name, type, institution_name, mask, currency, nickname FROM accounts")?;
        snap.accounts = s
            .query_map([], |r| {
                Ok(RawAccount {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    display_name: r.get(2)?,
                    kind: r.get(3)?,
                    institution: r.get(4)?,
                    mask: r.get(5)?,
                    currency: r.get(6)?,
                    nickname: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        if migrations::table_exists(&conn, "account_balances")? {
            let mut s = conn.prepare(
                "SELECT b.account_id, b.balance FROM account_balances b
                 WHERE b.balance_date = (SELECT max(b2.balance_date) FROM account_balances b2 WHERE b2.account_id = b.account_id)",
            )?;
            snap.balances = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        }

        let mut s = conn.prepare(
            "SELECT t.id, t.date, t.payee, t.original_name, t.notes, t.amount, t.currency, t.status, t.source,
                    t.tag_ids, t.recurring_id, t.external_id, t.created_at_lm, t.updated_at_lm, t.is_pending,
                    t.category_id, t.category_local_id, c.name,
                    COALESCE(a.id, m.id),
                    COALESCE(NULLIF(a.nickname,''), NULLIF(a.display_name,''), a.name, NULLIF(m.nickname,''), NULLIF(m.display_name,''), m.name),
                    t.user_notes
             FROM transactions t
             LEFT JOIN accounts a ON a.id = t.plaid_account_id
             LEFT JOIN accounts m ON m.id = t.manual_account_id
             LEFT JOIN categories c ON c.id = t.category_id
             ORDER BY t.id",
        )?;
        snap.txns = s
            .query_map([], |r| {
                Ok(RawTxn {
                    id: r.get(0)?,
                    date: r.get(1)?,
                    payee: r.get(2)?,
                    original_name: r.get(3)?,
                    notes: r.get(4)?,
                    amount: r.get(5)?,
                    currency: r.get(6)?,
                    status: r.get(7)?,
                    source: r.get(8)?,
                    tag_ids: r.get(9)?,
                    recurring_id: r.get(10)?,
                    external_id: r.get(11)?,
                    created_at_lm: r.get(12)?,
                    updated_at_lm: r.get(13)?,
                    is_pending: r.get::<_, i64>(14)? != 0,
                    category_id: r.get(15)?,
                    category_local_id: r.get(16)?,
                    lm_category: r.get(17)?,
                    account_id: r.get(18)?,
                    account: r.get(19)?,
                    user_notes: r.get(20)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(snap)
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
pub(crate) mod tests;
