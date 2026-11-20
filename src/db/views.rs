//! Saved views (`saved_view`): a name, a slash-search query and an optional sort. Writes are bound
//! statements inside one transaction that also journals a `change_batch` (kind `view`) with one
//! `change_item` (entity `saved_view`, field `row`, old/new value = the row as JSON, NULL when the
//! row did not exist / no longer exists), so a later undo phase can reverse them.

use super::Db;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub id: i64,
    pub name: String,
    pub query: String,
    pub sort: Option<String>,
    pub position: i64,
}

impl View {
    pub fn to_json(&self) -> serde_json::Value {
        json!({ "id": self.id, "name": self.name, "query": self.query, "sort": self.sort, "position": self.position })
    }

    /// The journal form: no id (it is the item's `entity_id`).
    fn journal(&self) -> String {
        json!({ "name": self.name, "query": self.query, "sort": self.sort, "position": self.position }).to_string()
    }
}

fn read_all(conn: &Connection) -> Result<Vec<View>> {
    let mut s = conn.prepare("SELECT id, name, COALESCE(query,''), sort, position FROM saved_view ORDER BY position, name COLLATE NOCASE, id")?;
    let rows = s
        .query_map([], |r| Ok(View { id: r.get(0)?, name: r.get(1)?, query: r.get(2)?, sort: r.get(3)?, position: r.get(4)? }))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn read_one(conn: &Connection, id: i64) -> Result<Option<View>> {
    Ok(conn
        .query_row("SELECT id, name, COALESCE(query,''), sort, position FROM saved_view WHERE id=?1", [id], |r| {
            Ok(View { id: r.get(0)?, name: r.get(1)?, query: r.get(2)?, sort: r.get(3)?, position: r.get(4)? })
        })
        .optional()?)
}

fn journal(conn: &Connection, label: &str, id: i64, old: Option<String>, new: Option<String>) -> Result<()> {
    conn.execute("INSERT INTO change_batch(label, kind) VALUES (?1, 'view')", [label])?;
    let batch = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO change_item(batch_id, entity, entity_id, field, old_value, new_value) VALUES (?1, 'saved_view', ?2, 'row', ?3, ?4)",
        params![batch, id, old, new],
    )?;
    Ok(())
}

impl Db {
    /// All views, ordered by position then name.
    pub fn views(&self) -> Result<Vec<View>> {
        read_all(&*self.lock()?)
    }

    /// Insert, or update the view whose name matches case-insensitively (the new spelling of the name
    /// wins, the position is kept). New views go to the end.
    pub fn view_save(&self, name: &str, query: &str, sort: Option<&str>) -> Result<View> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let lower = name.to_lowercase();
        let existing = read_all(&tx)?.into_iter().find(|v| v.name.to_lowercase() == lower);
        let saved = match existing {
            Some(old) => {
                tx.execute("UPDATE saved_view SET name=?1, query=?2, sort=?3 WHERE id=?4", params![name, query, sort, old.id])?;
                let new = View { id: old.id, name: name.to_string(), query: query.to_string(), sort: sort.map(str::to_string), position: old.position };
                journal(&tx, "update view", new.id, Some(old.journal()), Some(new.journal()))?;
                new
            }
            None => {
                let position: i64 = tx.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM saved_view", [], |r| r.get(0))?;
                tx.execute("INSERT INTO saved_view(name, query, sort, position) VALUES (?1, ?2, ?3, ?4)", params![name, query, sort, position])?;
                let new = View { id: tx.last_insert_rowid(), name: name.to_string(), query: query.to_string(), sort: sort.map(str::to_string), position };
                journal(&tx, "add view", new.id, None, Some(new.journal()))?;
                new
            }
        };
        tx.commit()?;
        Ok(saved)
    }

    /// Delete a view. `false` when there is no such id (nothing is journaled then).
    pub fn view_delete(&self, id: i64) -> Result<bool> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let Some(old) = read_one(&tx, id)? else {
            return Ok(false);
        };
        tx.execute("DELETE FROM saved_view WHERE id=?1", [id])?;
        journal(&tx, "delete view", id, Some(old.journal()), None)?;
        tx.commit()?;
        Ok(true)
    }
}
