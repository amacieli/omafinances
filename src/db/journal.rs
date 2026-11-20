//! Undo/redo journal. Every mutation that changes the database writes one `change_batch` with one
//! `change_item` per row write. An item holds the row's state before and after the write, as JSON, so a
//! batch can be reversed exactly.
//!
//! Entities and their states (`NULL` in the table = `None` here, meaning the row is absent):
//! - `category` / `row`: the full `category` row (all columns).
//! - `rule` / `row`: the full `rule` row.
//! - `saved_view` / `row`: `{name, query, sort, position}` (see `views.rs`).
//! - `transaction` / `category_local_id`: the local category id, or JSON `null`.
//!
//! Stack model: undo reverses the newest batch with `undone_at IS NULL`. Redo re-applies the most recently
//! undone batch, which is the lowest id among undone, non-superseded batches (undo always works down from the
//! top, so undone batches form a contiguous block above the live ones). Creating any batch supersedes every
//! undone batch. Undo and redo check the database first and refuse (409, nothing changed) when a row no
//! longer holds the state the journal expects.
//!
//! Writes are applied one row at a time in execution order. Undo walks the items backwards and redo walks them
//! forwards, so every intermediate state is one the original mutation passed through (this keeps the
//! unique (parent, name) index and the foreign keys satisfied during reversal).

use super::{categories, views, Db};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Why a mutation was refused. The API maps these to HTTP codes.
#[derive(Debug)]
pub enum MutError {
    /// 400: bad input.
    Invalid(String),
    /// 404: unknown id (the message names the kind of thing).
    NotFound(&'static str),
    /// 409: name clash, cycle, data changed underneath an undo, etc.
    Conflict(String),
    /// 409: a configured limit (category depth) would be exceeded.
    Limit(String),
    /// 409: undo or redo with nothing to act on.
    NothingToUndo,
    NothingToRedo,
    /// Unexpected database failure. Detail is logged by the caller, never returned.
    Other(anyhow::Error),
}

impl From<rusqlite::Error> for MutError {
    fn from(e: rusqlite::Error) -> Self {
        match &e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation => {
                MutError::Conflict("the change clashes with existing data; nothing was changed".into())
            }
            _ => MutError::Other(e.into()),
        }
    }
}

impl From<anyhow::Error> for MutError {
    fn from(e: anyhow::Error) -> Self {
        MutError::Other(e)
    }
}

/// Result of a mutation. `batch_id` is `None` when nothing changed (no batch is written then).
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub id: Option<i64>,
    pub batch_id: Option<i64>,
    pub label: String,
}

impl Outcome {
    pub fn unchanged() -> Outcome {
        Outcome { id: None, batch_id: None, label: "No change".into() }
    }
}

/// One row write as recorded in the journal.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub entity: String,
    pub entity_id: i64,
    pub field: String,
    pub old: Option<Value>,
    pub new: Option<Value>,
}

impl Item {
    pub fn new(entity: &str, entity_id: i64, field: &str, old: Option<Value>, new: Option<Value>) -> Item {
        Item { entity: entity.to_string(), entity_id, field: field.to_string(), old, new }
    }
}

/// Items collected while a mutation runs inside its transaction. Written as one batch at the end.
#[derive(Debug, Default)]
pub struct Pending {
    items: Vec<Item>,
}

impl Pending {
    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Write the batch (if anything changed): supersede the undone batches, then insert the batch and its
    /// items. Returns the batch id, or `None` for an empty batch. Call inside the mutation's transaction.
    pub fn commit(self, c: &Connection, label: &str, kind: &str) -> Result<Option<i64>> {
        if self.items.is_empty() {
            return Ok(None);
        }
        c.execute("UPDATE change_batch SET superseded=1 WHERE undone_at IS NOT NULL AND superseded=0", [])?;
        c.execute("INSERT INTO change_batch(label, kind) VALUES (?1, ?2)", params![label, kind])?;
        let batch = c.last_insert_rowid();
        let mut ins = c.prepare(
            "INSERT INTO change_item(batch_id, entity, entity_id, field, old_value, new_value) VALUES (?1,?2,?3,?4,?5,?6)",
        )?;
        for it in &self.items {
            ins.execute(params![batch, it.entity, it.entity_id, it.field, text_of(&it.old), text_of(&it.new)])?;
        }
        Ok(Some(batch))
    }
}

fn text_of(v: &Option<Value>) -> Option<String> {
    v.as_ref().map(Value::to_string)
}

fn parse_of(s: Option<String>) -> Result<Option<Value>> {
    match s {
        None => Ok(None),
        Some(t) => Ok(Some(serde_json::from_str(&t)?)),
    }
}

/// Current state of one journaled row, `None` when the row is absent.
pub fn read_state(c: &Connection, entity: &str, id: i64) -> Result<Option<Value>> {
    match entity {
        "category" => categories::category_json(c, id),
        "rule" => categories::rule_json(c, id),
        "saved_view" => views::row_json(c, id),
        "transaction" => {
            let v: Option<Option<i64>> = c
                .query_row("SELECT category_local_id FROM transactions WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            Ok(v.map(|x| json!(x)))
        }
        other => bail!("unknown journal entity {other}"),
    }
}

/// Force a row into `v` (`None` = absent). Used by undo and redo only.
pub fn apply(c: &Connection, entity: &str, id: i64, v: Option<&Value>) -> Result<()> {
    match entity {
        "category" => categories::write_category_json(c, id, v),
        "rule" => categories::write_rule_json(c, id, v),
        "saved_view" => views::write_row(c, id, v),
        "transaction" => {
            let Some(v) = v else { bail!("transaction rows cannot be absent") };
            let n = c.execute("UPDATE transactions SET category_local_id=?1 WHERE id=?2", params![v.as_i64(), id])?;
            if n != 1 {
                bail!("transaction {id} is missing");
            }
            Ok(())
        }
        other => bail!("unknown journal entity {other}"),
    }
}

/// Refuse when the database is not in the state the journal expects. Undo expects each row's last `new`,
/// redo expects each row's first `old`.
fn check(c: &Connection, items: &[Item], undo: bool) -> Result<(), MutError> {
    let mut expect: BTreeMap<(String, i64, String), Option<Value>> = BTreeMap::new();
    for it in items {
        let key = (it.entity.clone(), it.entity_id, it.field.clone());
        if undo {
            expect.insert(key, it.new.clone());
        } else {
            expect.entry(key).or_insert_with(|| it.old.clone());
        }
    }
    for ((entity, id, _), want) in expect {
        let now = read_state(c, &entity, id).map_err(|_| conflict())?;
        if now != want {
            return Err(conflict());
        }
    }
    Ok(())
}

fn conflict() -> MutError {
    MutError::Conflict("the data changed since this change was made; nothing was changed".into())
}

fn load_items(c: &Connection, batch: i64) -> Result<Vec<Item>> {
    let mut s = c.prepare(
        "SELECT entity, entity_id, COALESCE(field,''), old_value, new_value FROM change_item WHERE batch_id=?1 ORDER BY id",
    )?;
    let rows = s.query_map([batch], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (entity, entity_id, field, old, new) = row?;
        out.push(Item { entity, entity_id, field, old: parse_of(old)?, new: parse_of(new)? });
    }
    Ok(out)
}

/// Undo and redo share this: `undo` picks the newest live batch, `redo` the most recently undone one.
fn reverse(c: &mut Connection, undo: bool) -> Result<Outcome, MutError> {
    let tx = c.transaction()?;
    let pick = if undo {
        "SELECT id, label FROM change_batch WHERE undone_at IS NULL ORDER BY id DESC LIMIT 1"
    } else {
        "SELECT id, label FROM change_batch WHERE undone_at IS NOT NULL AND superseded=0 ORDER BY id ASC LIMIT 1"
    };
    let picked: Option<(i64, String)> = tx.query_row(pick, [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((batch, label)) = picked else {
        return Err(if undo { MutError::NothingToUndo } else { MutError::NothingToRedo });
    };
    let items = load_items(&tx, batch)?;
    check(&tx, &items, undo)?;
    // A failure here (should not happen after the check) rolls the whole thing back.
    let apply_all = || -> Result<()> {
        if undo {
            for it in items.iter().rev() {
                apply(&tx, &it.entity, it.entity_id, it.old.as_ref())?;
            }
            tx.execute("UPDATE change_batch SET undone_at=datetime('now') WHERE id=?1", [batch])?;
        } else {
            for it in &items {
                apply(&tx, &it.entity, it.entity_id, it.new.as_ref())?;
            }
            tx.execute("UPDATE change_batch SET undone_at=NULL WHERE id=?1", [batch])?;
        }
        Ok(())
    };
    apply_all().map_err(|_| MutError::Conflict("the change could not be applied; nothing was changed".into()))?;
    tx.commit()?;
    Ok(Outcome { id: None, batch_id: Some(batch), label })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JournalRow {
    pub id: i64,
    pub created_at: String,
    pub label: String,
    pub kind: String,
    pub undone: bool,
    pub superseded: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JournalView {
    pub rows: Vec<JournalRow>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub next_undo_label: Option<String>,
    pub next_redo_label: Option<String>,
}

impl Db {
    /// Reverse the newest live batch.
    pub fn undo(&self) -> Result<Outcome, MutError> {
        let mut conn = self.lock()?;
        reverse(&mut conn, true)
    }

    /// Re-apply the most recently undone batch, if it has not been superseded.
    pub fn redo(&self) -> Result<Outcome, MutError> {
        let mut conn = self.lock()?;
        reverse(&mut conn, false)
    }

    /// Newest batches first.
    pub fn journal(&self, limit: i64) -> Result<JournalView> {
        let conn = self.lock()?;
        let mut s = conn.prepare(
            "SELECT id, COALESCE(created_at,''), COALESCE(label,''), COALESCE(kind,''), undone_at IS NOT NULL, superseded
             FROM change_batch ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = s
            .query_map([limit.clamp(1, 100)], |r| {
                Ok(JournalRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    label: r.get(2)?,
                    kind: r.get(3)?,
                    undone: r.get::<_, i64>(4)? != 0,
                    superseded: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let label = |sql: &str| -> Result<Option<String>> { Ok(conn.query_row(sql, [], |r| r.get(0)).optional()?) };
        let next_undo_label = label("SELECT label FROM change_batch WHERE undone_at IS NULL ORDER BY id DESC LIMIT 1")?;
        let next_redo_label =
            label("SELECT label FROM change_batch WHERE undone_at IS NOT NULL AND superseded=0 ORDER BY id ASC LIMIT 1")?;
        Ok(JournalView {
            rows,
            can_undo: next_undo_label.is_some(),
            can_redo: next_redo_label.is_some(),
            next_undo_label,
            next_redo_label,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
