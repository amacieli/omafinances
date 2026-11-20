//! Local category tree and manual categorization. Every public method runs in one SQLite transaction, writes
//! one change batch (see `journal.rs`) and returns the batch id. Nothing here touches Lunch Money data
//! except `transactions.category_local_id` and `rule.category_id`.
//!
//! Conventions:
//! - Depth counts from 1 at the top level. `max_depth` comes from settings and is checked where a node gets
//!   a new place (create, move, merge); the depth of a moved subtree is checked to its deepest node.
//! - Display order is parent, then siblings by `sort`, then name, then id (the same order as `cache.rs`).
//!   Renumbering makes `sort` dense from 0 for the affected siblings.
//! - Names are trimmed, 1..=80 characters, no control characters, unique among siblings case-insensitively.
//! - A change that leaves the data as it was writes no batch and returns `Outcome::unchanged()`.

use super::journal::{Item, MutError, Outcome, Pending};
use super::Db;
use crate::cache::PATH_SEP;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_CATEGORY_IDS: usize = 5000;
pub const KINDS: [&str; 3] = ["expense", "income", "transfer"];
/// Bound on parent-chain walks, so a corrupt cycle in the data cannot hang a request.
const MAX_WALK: usize = 256;

/// One `category` row, all columns. This is also the journal form.
#[derive(Debug, Clone, PartialEq)]
pub struct CatRow {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub sort: Option<i64>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub legacy_table: Option<String>,
    pub legacy_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl CatRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "parent_id": self.parent_id, "name": self.name, "kind": self.kind, "sort": self.sort,
            "color": self.color, "icon": self.icon, "legacy_table": self.legacy_table, "legacy_id": self.legacy_id,
            "created_at": self.created_at, "updated_at": self.updated_at,
        })
    }

    pub fn from_json(v: &Value) -> Result<CatRow> {
        let int = |k: &str| v.get(k).and_then(Value::as_i64);
        let txt = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Ok(CatRow {
            id: int("id").ok_or_else(|| anyhow!("category journal row without id"))?,
            parent_id: int("parent_id"),
            name: txt("name").ok_or_else(|| anyhow!("category journal row without name"))?,
            kind: txt("kind").ok_or_else(|| anyhow!("category journal row without kind"))?,
            sort: int("sort"),
            color: txt("color"),
            icon: txt("icon"),
            legacy_table: txt("legacy_table"),
            legacy_id: int("legacy_id"),
            created_at: txt("created_at").unwrap_or_default(),
            updated_at: txt("updated_at").unwrap_or_default(),
        })
    }
}

/// Trim and check a category name. Returns the trimmed name.
pub fn validate_name(raw: &str) -> Result<String, MutError> {
    let n = raw.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME_CHARS {
        return Err(MutError::Invalid(format!("name must be 1 to {MAX_NAME_CHARS} characters")));
    }
    if n.chars().any(char::is_control) {
        return Err(MutError::Invalid("name must not contain control characters".into()));
    }
    Ok(n.to_string())
}

pub fn valid_kind(k: &str) -> Result<String, MutError> {
    if KINDS.contains(&k) {
        Ok(k.to_string())
    } else {
        Err(MutError::Invalid("kind must be expense, income or transfer".into()))
    }
}

// ---- reads and journal glue (used by journal.rs) -------------------------------------------------

pub(crate) fn load(c: &Connection, id: i64) -> Result<Option<CatRow>> {
    Ok(c.query_row(
        "SELECT id, parent_id, name, kind, sort, color, icon, legacy_table, legacy_id, created_at, updated_at
         FROM category WHERE id=?1",
        [id],
        |r| {
            Ok(CatRow {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                sort: r.get(4)?,
                color: r.get(5)?,
                icon: r.get(6)?,
                legacy_table: r.get(7)?,
                legacy_id: r.get(8)?,
                created_at: r.get(9)?,
                updated_at: r.get(10)?,
            })
        },
    )
    .optional()?)
}

fn must(c: &Connection, id: i64) -> Result<CatRow, MutError> {
    load(c, id)?.ok_or(MutError::NotFound("category not found"))
}

fn put_category(c: &Connection, r: &CatRow) -> Result<()> {
    c.execute(
        "INSERT INTO category(id, parent_id, name, kind, sort, color, icon, legacy_table, legacy_id, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(id) DO UPDATE SET parent_id=excluded.parent_id, name=excluded.name, kind=excluded.kind,
            sort=excluded.sort, color=excluded.color, icon=excluded.icon, legacy_table=excluded.legacy_table,
            legacy_id=excluded.legacy_id, created_at=excluded.created_at, updated_at=excluded.updated_at",
        params![r.id, r.parent_id, r.name, r.kind, r.sort, r.color, r.icon, r.legacy_table, r.legacy_id, r.created_at, r.updated_at],
    )?;
    Ok(())
}

pub(crate) fn category_json(c: &Connection, id: i64) -> Result<Option<Value>> {
    Ok(load(c, id)?.map(|r| r.to_json()))
}

pub(crate) fn write_category_json(c: &Connection, id: i64, v: Option<&Value>) -> Result<()> {
    match v {
        None => {
            c.execute("DELETE FROM category WHERE id=?1", [id])?;
        }
        Some(v) => {
            let r = CatRow::from_json(v)?;
            if r.id != id {
                bail!("category journal row id mismatch");
            }
            put_category(c, &r)?;
        }
    }
    Ok(())
}

pub(crate) fn rule_json(c: &Connection, id: i64) -> Result<Option<Value>> {
    Ok(c.query_row(
        "SELECT id, position, name, enabled, match_json, category_id, hits, created_at FROM rule WHERE id=?1",
        [id],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "position": r.get::<_, i64>(1)?, "name": r.get::<_, Option<String>>(2)?,
                "enabled": r.get::<_, i64>(3)?, "match_json": r.get::<_, String>(4)?,
                "category_id": r.get::<_, Option<i64>>(5)?, "hits": r.get::<_, i64>(6)?,
                "created_at": r.get::<_, String>(7)?,
            }))
        },
    )
    .optional()?)
}

pub(crate) fn write_rule_json(c: &Connection, id: i64, v: Option<&Value>) -> Result<()> {
    let Some(v) = v else {
        c.execute("DELETE FROM rule WHERE id=?1", [id])?;
        return Ok(());
    };
    let int = |k: &str| v.get(k).and_then(Value::as_i64);
    let txt = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    c.execute(
        "INSERT INTO rule(id, position, name, enabled, match_json, category_id, hits, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(id) DO UPDATE SET position=excluded.position, name=excluded.name, enabled=excluded.enabled,
            match_json=excluded.match_json, category_id=excluded.category_id, hits=excluded.hits, created_at=excluded.created_at",
        params![
            id,
            int("position").unwrap_or(0),
            txt("name"),
            int("enabled").unwrap_or(1),
            txt("match_json").unwrap_or_else(|| "{}".into()),
            int("category_id"),
            int("hits").unwrap_or(0),
            txt("created_at").unwrap_or_default(),
        ],
    )?;
    Ok(())
}

// ---- tree queries ---------------------------------------------------------------------------------

fn now(c: &Connection) -> Result<String> {
    Ok(c.query_row("SELECT datetime('now')", [], |r| r.get(0))?)
}

/// Children of `parent` (`None` = top level) in display order.
fn children(c: &Connection, parent: Option<i64>) -> Result<Vec<CatRow>> {
    let mut s = c.prepare(
        "SELECT id FROM category WHERE parent_id IS ?1 ORDER BY COALESCE(sort,0), name, id",
    )?;
    let ids: Vec<i64> = s.query_map([parent], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(r) = load(c, id)? {
            out.push(r);
        }
    }
    Ok(out)
}

fn child_ids(c: &Connection, parent: Option<i64>) -> Result<Vec<i64>> {
    Ok(children(c, parent)?.into_iter().map(|r| r.id).collect())
}

fn parent_of(c: &Connection, id: i64) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT parent_id FROM category WHERE id=?1", [id], |r| r.get::<_, Option<i64>>(0))
        .optional()?
        .flatten())
}

/// Level of a node, top = 1.
fn depth_of(c: &Connection, id: i64) -> Result<u32> {
    let mut depth = 1;
    let mut cur = id;
    for _ in 0..MAX_WALK {
        match parent_of(c, cur)? {
            Some(p) => {
                depth += 1;
                cur = p;
            }
            None => break,
        }
    }
    Ok(depth)
}

/// True when `cur` is `root` or one of its descendants.
fn is_within(c: &Connection, cur: i64, root: i64) -> Result<bool> {
    let mut at = cur;
    for _ in 0..MAX_WALK {
        if at == root {
            return Ok(true);
        }
        match parent_of(c, at)? {
            Some(p) => at = p,
            None => return Ok(false),
        }
    }
    Ok(false)
}

/// Levels below `id`: 0 for a leaf.
fn height(c: &Connection, id: i64) -> Result<u32> {
    fn go(c: &Connection, id: i64, budget: usize) -> Result<u32> {
        if budget == 0 {
            bail!("category tree is deeper than the walk limit");
        }
        let mut h = 0;
        for k in child_ids(c, Some(id))? {
            h = h.max(1 + go(c, k, budget - 1)?);
        }
        Ok(h)
    }
    go(c, id, MAX_WALK)
}

pub(crate) fn path_of(c: &Connection, id: i64) -> Result<String> {
    let mut names = Vec::new();
    let mut cur = Some(id);
    for _ in 0..MAX_WALK {
        let Some(i) = cur else { break };
        let Some(r) = load(c, i)? else { break };
        names.push(r.name);
        cur = r.parent_id;
    }
    names.reverse();
    Ok(names.join(PATH_SEP))
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn clash(c: &Connection, parent: Option<i64>, name: &str, except: Option<i64>) -> Result<bool> {
    Ok(children(c, parent)?.iter().any(|r| Some(r.id) != except && same_name(&r.name, name)))
}

fn find_name<'a>(list: &'a [CatRow], name: &str) -> Option<&'a CatRow> {
    list.iter().find(|r| same_name(&r.name, name))
}

// ---- journaled writes -----------------------------------------------------------------------------

/// Save a row if anything but `updated_at` changed; journal the before and after.
fn save(c: &Connection, p: &mut Pending, mut row: CatRow) -> Result<()> {
    let old = load(c, row.id)?.ok_or_else(|| anyhow!("category row vanished"))?;
    if old == row {
        return Ok(());
    }
    row.updated_at = now(c)?;
    put_category(c, &row)?;
    p.push(Item::new("category", row.id, "row", Some(old.to_json()), Some(row.to_json())));
    Ok(())
}

fn set_sort(c: &Connection, p: &mut Pending, id: i64, sort: i64) -> Result<()> {
    let mut row = load(c, id)?.ok_or_else(|| anyhow!("category row vanished"))?;
    row.sort = Some(sort);
    save(c, p, row)
}

/// Make `order` the dense sibling order: the i-th id gets sort i.
fn renumber(c: &Connection, p: &mut Pending, order: &[i64]) -> Result<()> {
    for (i, id) in order.iter().enumerate() {
        set_sort(c, p, *id, i as i64)?;
    }
    Ok(())
}

fn remove(c: &Connection, p: &mut Pending, id: i64) -> Result<()> {
    let old = load(c, id)?.ok_or_else(|| anyhow!("category row vanished"))?;
    c.execute("DELETE FROM category WHERE id=?1", [id])?;
    p.push(Item::new("category", id, "row", Some(old.to_json()), None));
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxnSet {
    Changed,
    Same,
    Missing,
}

/// Set one transaction's local category. Journals only real changes.
pub(crate) fn set_txn(c: &Connection, p: &mut Pending, txn: i64, cat: Option<i64>) -> Result<TxnSet> {
    let cur: Option<Option<i64>> = c
        .query_row("SELECT category_local_id FROM transactions WHERE id=?1", [txn], |r| r.get(0))
        .optional()?;
    let Some(cur) = cur else { return Ok(TxnSet::Missing) };
    if cur == cat {
        return Ok(TxnSet::Same);
    }
    c.execute("UPDATE transactions SET category_local_id=?1 WHERE id=?2", params![cat, txn])?;
    p.push(Item::new("transaction", txn, "category_local_id", Some(json!(cur)), Some(json!(cat))));
    Ok(TxnSet::Changed)
}

fn txn_ids_in(c: &Connection, cat: i64) -> Result<Vec<i64>> {
    let mut s = c.prepare("SELECT id FROM transactions WHERE category_local_id=?1 ORDER BY id")?;
    let ids = s.query_map([cat], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

fn rule_ids_in(c: &Connection, cat: i64) -> Result<Vec<i64>> {
    let mut s = c.prepare("SELECT id FROM rule WHERE category_id=?1 ORDER BY id")?;
    let ids = s.query_map([cat], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

fn set_rule_cat(c: &Connection, p: &mut Pending, rule: i64, cat: Option<i64>) -> Result<()> {
    let old = rule_json(c, rule)?.ok_or_else(|| anyhow!("rule row vanished"))?;
    c.execute("UPDATE rule SET category_id=?1 WHERE id=?2", params![cat, rule])?;
    let new = rule_json(c, rule)?;
    if new.as_ref() != Some(&old) {
        p.push(Item::new("rule", rule, "row", Some(old), new));
    }
    Ok(())
}

// ---- structural operations (run inside a transaction) ---------------------------------------------

fn create_inner(c: &Connection, max_depth: u32, parent: Option<i64>, name: &str, kind: Option<&str>) -> Result<Outcome, MutError> {
    let parent_row = match parent {
        Some(pid) => Some(must(c, pid)?),
        None => None,
    };
    let depth = match parent {
        Some(pid) => depth_of(c, pid)? + 1,
        None => 1,
    };
    if depth > max_depth {
        return Err(MutError::Limit(format!("category depth limit is {max_depth} levels; this would be level {depth}")));
    }
    if clash(c, parent, name, None)? {
        return Err(MutError::Conflict("a category with that name already exists here".into()));
    }
    let kind = match kind {
        Some(k) => k.to_string(),
        None => parent_row.as_ref().map_or_else(|| "expense".to_string(), |p| p.kind.clone()),
    };
    let sort = children(c, parent)?.iter().map(|r| r.sort.unwrap_or(0)).max().map_or(0, |m| m + 1);
    c.execute(
        "INSERT INTO category(parent_id, name, kind, sort, created_at, updated_at) VALUES (?1,?2,?3,?4,datetime('now'),datetime('now'))",
        params![parent, name, kind, sort],
    )?;
    let id = c.last_insert_rowid();
    let row = must(c, id)?;
    let label = match &parent_row {
        Some(p) => format!("Create \"{}\" under \"{}\"", name, p.name),
        None => format!("Create \"{}\"", name),
    };
    let mut p = Pending::default();
    p.push(Item::new("category", id, "row", None, Some(row.to_json())));
    let batch_id = p.commit(c, &label, "category")?;
    Ok(Outcome { id: Some(id), batch_id, label })
}

fn rename_inner(c: &Connection, id: i64, name: &str) -> Result<Outcome, MutError> {
    let node = must(c, id)?;
    if node.name == name {
        return Ok(Outcome::unchanged());
    }
    if clash(c, node.parent_id, name, Some(id))? {
        return Err(MutError::Conflict("a sibling category already has that name".into()));
    }
    let label = format!("Rename \"{}\" to \"{}\"", node.name, name);
    let mut row = node;
    row.name = name.to_string();
    let mut p = Pending::default();
    save(c, &mut p, row)?;
    let batch_id = p.commit(c, &label, "category")?;
    Ok(Outcome { id: Some(id), batch_id, label })
}

fn set_kind_inner(c: &Connection, id: i64, kind: &str) -> Result<Outcome, MutError> {
    let node = must(c, id)?;
    if node.kind == kind {
        return Ok(Outcome::unchanged());
    }
    let label = format!("Set kind of \"{}\" to {}", node.name, kind);
    let mut row = node;
    row.kind = kind.to_string();
    let mut p = Pending::default();
    save(c, &mut p, row)?;
    let batch_id = p.commit(c, &label, "category")?;
    Ok(Outcome { id: Some(id), batch_id, label })
}

fn move_inner(c: &Connection, max_depth: u32, id: i64, new_parent: Option<i64>, position: Option<usize>) -> Result<Outcome, MutError> {
    let node = must(c, id)?;
    let parent_name = match new_parent {
        Some(np) => {
            let row = must(c, np)?;
            if is_within(c, np, id)? {
                return Err(MutError::Conflict("a category cannot move under itself or one of its descendants".into()));
            }
            Some(row.name)
        }
        None => None,
    };
    let new_depth = match new_parent {
        Some(np) => depth_of(c, np)? + 1,
        None => 1,
    };
    let deepest = new_depth + height(c, id)?;
    if deepest > max_depth {
        return Err(MutError::Limit(format!(
            "category depth limit is {max_depth} levels; this move would make the tree {deepest} levels deep"
        )));
    }
    if clash(c, new_parent, &node.name, Some(id))? {
        return Err(MutError::Conflict("a category with that name already exists there".into()));
    }
    let old_parent = node.parent_id;
    let label = if old_parent != new_parent {
        match &parent_name {
            Some(n) => format!("Move \"{}\" under \"{}\"", node.name, n),
            None => format!("Move \"{}\" to the top level", node.name),
        }
    } else {
        format!("Reorder \"{}\"", node.name)
    };

    let mut p = Pending::default();
    if old_parent != new_parent {
        let rest: Vec<i64> = child_ids(c, old_parent)?.into_iter().filter(|&x| x != id).collect();
        renumber(c, &mut p, &rest)?;
    }
    let mut moved = node;
    moved.parent_id = new_parent;
    save(c, &mut p, moved)?;
    let mut order: Vec<i64> = child_ids(c, new_parent)?.into_iter().filter(|&x| x != id).collect();
    let at = position.unwrap_or(order.len()).min(order.len());
    order.insert(at, id);
    renumber(c, &mut p, &order)?;
    let batch_id = p.commit(c, &label, "category")?;
    Ok(Outcome { id: Some(id), batch_id, label })
}

fn move_within_inner(c: &Connection, max_depth: u32, id: i64, delta: i64) -> Result<Outcome, MutError> {
    if delta != -1 && delta != 1 {
        return Err(MutError::Invalid("delta must be -1 or 1".into()));
    }
    let node = must(c, id)?;
    let siblings = child_ids(c, node.parent_id)?;
    let i = siblings.iter().position(|&x| x == id).unwrap_or(0) as i64;
    let target = i + delta;
    if target < 0 || target >= siblings.len() as i64 {
        return Ok(Outcome { id: Some(id), ..Outcome::unchanged() });
    }
    // Position counts among the siblings without `id`: moving one step puts it at i+delta.
    move_inner(c, max_depth, id, node.parent_id, Some(target as usize))
}

fn delete_inner(c: &Connection, id: i64, reassign_to: Option<i64>) -> Result<Outcome, MutError> {
    let node = must(c, id)?;
    let target = match reassign_to {
        Some(r) => {
            if is_within(c, r, id)? {
                return Err(MutError::Conflict(
                    "transactions cannot move to the category being deleted or one of its subcategories".into(),
                ));
            }
            Some(must(c, r)?)
        }
        None => None,
    };
    let dest = node.parent_id;
    let kids = children(c, Some(id))?;
    let others: Vec<CatRow> = children(c, dest)?.into_iter().filter(|r| r.id != id).collect();
    for k in &kids {
        if find_name(&others, &k.name).is_some() {
            return Err(MutError::Conflict(format!(
                "cannot move \"{}\" up: a sibling already has that name",
                k.name
            )));
        }
    }

    let mut p = Pending::default();
    let mut moved = 0usize;
    for t in txn_ids_in(c, id)? {
        if set_txn(c, &mut p, t, reassign_to)? == TxnSet::Changed {
            moved += 1;
        }
    }
    for r in rule_ids_in(c, id)? {
        set_rule_cat(c, &mut p, r, reassign_to)?;
    }
    let mut order: Vec<i64> = others.iter().map(|r| r.id).collect();
    for k in kids {
        order.push(k.id);
        let mut row = k;
        row.parent_id = dest;
        save(c, &mut p, row)?;
    }
    renumber(c, &mut p, &order)?;
    remove(c, &mut p, id)?;

    let plural = if moved == 1 { "transaction" } else { "transactions" };
    let mut label = format!("Delete \"{}\"", node.name);
    if moved > 0 {
        match &target {
            Some(t) => label.push_str(&format!(" and move {moved} {plural} to \"{}\"", t.name)),
            None => label.push_str(&format!(" and uncategorize {moved} {plural}")),
        }
    }
    let batch_id = p.commit(c, &label, "category")?;
    Ok(Outcome { id: Some(id), batch_id, label })
}

/// Refuse a merge whose result would be deeper than `max_depth`. Mirrors `merge_rec`'s decisions.
fn merge_plan(c: &Connection, src: i64, dst: i64, dst_depth: u32, max_depth: u32) -> Result<(), MutError> {
    let dst_kids = children(c, Some(dst))?;
    for k in children(c, Some(src))? {
        match find_name(&dst_kids, &k.name) {
            Some(d) => merge_plan(c, k.id, d.id, dst_depth + 1, max_depth)?,
            None => {
                let deepest = dst_depth + 1 + height(c, k.id)?;
                if deepest > max_depth {
                    return Err(MutError::Limit(format!(
                        "category depth limit is {max_depth} levels; this merge would make the tree {deepest} levels deep"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Move everything of `src` into `dst`, then delete `src`. Same-named children merge recursively.
fn merge_rec(c: &Connection, p: &mut Pending, src: i64, dst: i64) -> Result<(), MutError> {
    for t in txn_ids_in(c, src)? {
        set_txn(c, p, t, Some(dst))?;
    }
    for r in rule_ids_in(c, src)? {
        set_rule_cat(c, p, r, Some(dst))?;
    }
    for k in children(c, Some(src))? {
        let dst_kids = children(c, Some(dst))?;
        match find_name(&dst_kids, &k.name).map(|d| d.id) {
            Some(d) => merge_rec(c, p, k.id, d)?,
            None => {
                let sort = dst_kids.iter().map(|r| r.sort.unwrap_or(0)).max().map_or(0, |m| m + 1);
                let mut row = k;
                row.parent_id = Some(dst);
                row.sort = Some(sort);
                save(c, p, row)?;
            }
        }
    }
    let order = child_ids(c, Some(dst))?;
    renumber(c, p, &order)?;
    remove(c, p, src)?;
    Ok(())
}

fn merge_inner(c: &Connection, max_depth: u32, src: i64, into: i64) -> Result<Outcome, MutError> {
    let s = must(c, src)?;
    let t = must(c, into)?;
    if src == into {
        return Err(MutError::Conflict("cannot merge a category into itself".into()));
    }
    if is_within(c, into, src)? {
        return Err(MutError::Conflict("cannot merge a category into one of its own subcategories".into()));
    }
    let dst_depth = depth_of(c, into)?;
    merge_plan(c, src, into, dst_depth, max_depth)?;
    let label = format!("Merge \"{}\" into \"{}\"", s.name, t.name);
    let mut p = Pending::default();
    merge_rec(c, &mut p, src, into)?;
    let batch_id = p.commit(c, &label, "category")?;
    Ok(Outcome { id: Some(src), batch_id, label })
}

/// Result of a manual categorization.
#[derive(Debug, Clone, PartialEq)]
pub struct Categorized {
    pub batch_id: Option<i64>,
    pub changed: usize,
    /// Unknown ids, plus ids whose category already was the target.
    pub skipped: usize,
    pub label: String,
}

// ---- public mutations -----------------------------------------------------------------------------

impl Db {
    /// Run one mutation in its own transaction.
    fn mutate<T>(&self, f: impl FnOnce(&Connection) -> Result<T, MutError>) -> Result<T, MutError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Add a category. Kind defaults to the parent's kind (expense at the top level).
    pub fn create_category(&self, max_depth: u32, parent: Option<i64>, name: &str, kind: Option<&str>) -> Result<Outcome, MutError> {
        let name = validate_name(name)?;
        let kind = kind.map(valid_kind).transpose()?;
        self.mutate(|c| create_inner(c, max_depth, parent, &name, kind.as_deref()))
    }

    pub fn rename_category(&self, id: i64, name: &str) -> Result<Outcome, MutError> {
        let name = validate_name(name)?;
        self.mutate(|c| rename_inner(c, id, &name))
    }

    /// Change the kind of this node only (children keep theirs).
    pub fn set_category_kind(&self, id: i64, kind: &str) -> Result<Outcome, MutError> {
        let kind = valid_kind(kind)?;
        self.mutate(|c| set_kind_inner(c, id, &kind))
    }

    /// Re-parent and/or reorder. `position` counts among the new parent's other children (end when `None`).
    pub fn move_category(&self, max_depth: u32, id: i64, new_parent: Option<i64>, position: Option<usize>) -> Result<Outcome, MutError> {
        self.mutate(|c| move_inner(c, max_depth, id, new_parent, position))
    }

    /// Swap with the previous (`-1`) or next (`+1`) sibling. Already first or last: unchanged.
    pub fn move_category_within(&self, max_depth: u32, id: i64, delta: i64) -> Result<Outcome, MutError> {
        self.mutate(|c| move_within_inner(c, max_depth, id, delta))
    }

    /// Delete a node. Its own transactions and rules go to `reassign_to` (or become uncategorized); its
    /// children move up to its parent, after its siblings.
    pub fn delete_category(&self, id: i64, reassign_to: Option<i64>) -> Result<Outcome, MutError> {
        self.mutate(|c| delete_inner(c, id, reassign_to))
    }

    /// Fold `src` (and its subtree) into `into`.
    pub fn merge_category(&self, max_depth: u32, src: i64, into: i64) -> Result<Outcome, MutError> {
        self.mutate(|c| merge_inner(c, max_depth, src, into))
    }

    /// Set the local category of many transactions (`None` uncategorizes). Unknown ids and ids already in
    /// that category are counted as skipped.
    pub fn categorize(&self, ids: &[i64], category: Option<i64>) -> Result<Categorized, MutError> {
        if ids.is_empty() {
            return Err(MutError::Invalid("ids must not be empty".into()));
        }
        if ids.len() > MAX_CATEGORY_IDS {
            return Err(MutError::Invalid(format!("at most {MAX_CATEGORY_IDS} transaction ids per call")));
        }
        let mut uniq = ids.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        self.mutate(|c| {
            let path = match category {
                Some(cid) => {
                    must(c, cid)?;
                    Some(path_of(c, cid)?)
                }
                None => None,
            };
            let mut p = Pending::default();
            let (mut changed, mut skipped) = (0usize, 0usize);
            for t in uniq {
                match set_txn(c, &mut p, t, category)? {
                    TxnSet::Changed => changed += 1,
                    TxnSet::Same | TxnSet::Missing => skipped += 1,
                }
            }
            let plural = if changed == 1 { "transaction" } else { "transactions" };
            let label = match (changed, &path) {
                (0, _) => "No change".to_string(),
                (n, Some(p)) => format!("Categorize {n} {plural} as {p}"),
                (n, None) => format!("Uncategorize {n} {plural}"),
            };
            let batch_id = p.commit(c, &label, "categorize")?;
            Ok(Categorized { batch_id, changed, skipped, label })
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
