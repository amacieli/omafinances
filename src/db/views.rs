//! Saved views (`saved_view`): a name, a slash-search query and an optional sort. Writes are bound
//! statements inside one transaction that also journals one batch (kind `view`) with one `change_item`
//! (entity `saved_view`, field `row`, old/new = `{name, query, sort, position}` as JSON, NULL when the row
//! did not exist / no longer exists). The journal module reverses them.

use super::journal::{Item, Pending};
use super::Db;
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

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
    fn journal(&self) -> Value {
        json!({ "name": self.name, "query": self.query, "sort": self.sort, "position": self.position })
    }
}

/// The journal form of a view row, `None` when there is no such view.
pub(crate) fn row_json(c: &Connection, id: i64) -> Result<Option<Value>> {
    Ok(read_one(c, id)?.map(|v| v.journal()))
}

/// Force a view row to the journal form (`None` deletes it). Used by undo and redo.
pub(crate) fn write_row(c: &Connection, id: i64, v: Option<&Value>) -> Result<()> {
    let Some(v) = v else {
        c.execute("DELETE FROM saved_view WHERE id=?1", [id])?;
        return Ok(());
    };
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let name = text("name").ok_or_else(|| anyhow!("saved view journal row without a name"))?;
    let query = text("query").unwrap_or_default();
    let sort = text("sort");
    let position = v.get("position").and_then(Value::as_i64).unwrap_or(0);
    c.execute(
        "INSERT INTO saved_view(id, name, query, sort, position) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, query=excluded.query, sort=excluded.sort, position=excluded.position",
        params![id, name, query, sort, position],
    )?;
    Ok(())
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
        let mut pending = Pending::default();
        let lower = name.to_lowercase();
        let existing = read_all(&tx)?.into_iter().find(|v| v.name.to_lowercase() == lower);
        let (saved, label) = match existing {
            Some(old) => {
                tx.execute("UPDATE saved_view SET name=?1, query=?2, sort=?3 WHERE id=?4", params![name, query, sort, old.id])?;
                let new = View { id: old.id, name: name.to_string(), query: query.to_string(), sort: sort.map(str::to_string), position: old.position };
                pending.push(Item::new("saved_view", new.id, "row", Some(old.journal()), Some(new.journal())));
                (new, "update view")
            }
            None => {
                let position: i64 = tx.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM saved_view", [], |r| r.get(0))?;
                tx.execute("INSERT INTO saved_view(name, query, sort, position) VALUES (?1, ?2, ?3, ?4)", params![name, query, sort, position])?;
                let new = View { id: tx.last_insert_rowid(), name: name.to_string(), query: query.to_string(), sort: sort.map(str::to_string), position };
                pending.push(Item::new("saved_view", new.id, "row", None, Some(new.journal())));
                (new, "add view")
            }
        };
        pending.commit(&tx, label, "view")?;
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
        let mut pending = Pending::default();
        pending.push(Item::new("saved_view", id, "row", Some(old.journal()), None));
        pending.commit(&tx, "delete view", "view")?;
        tx.commit()?;
        Ok(true)
    }
}
