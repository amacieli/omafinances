//! Smart apply and the rule list. `smart_apply` writes the category of many rows and, optionally, one `rule` row, in
//! one SQLite transaction and one change batch of kind `smart`, so undo and redo cover both together.

use super::categories::{self, TxnSet};
use super::journal::{Item, MutError, Pending};
use super::Db;
use crate::smart::normalize::{normalize, AliasRow, Aliases};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Map, Value};

/// Most rows one apply may touch.
pub const MAX_APPLY_IDS: usize = 5000;

#[derive(Debug, Clone, PartialEq)]
pub struct SmartApplied {
    pub batch_id: Option<i64>,
    pub label: String,
    /// Rows whose category was written.
    pub changed: usize,
    /// Unknown ids, and rows already in the target category.
    pub skipped: usize,
    /// Rows already in a different category, left alone because `overwrite` is false.
    pub skipped_conflicts: usize,
    pub rule_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuleRow {
    pub id: i64,
    pub position: i64,
    pub name: Option<String>,
    pub enabled: bool,
    #[serde(rename = "match")]
    pub matched: Value,
    pub category_id: Option<i64>,
    pub category_path: Option<String>,
    pub hits: i64,
}

/// `payee_alias` rows, compiled. Read inside the caller's connection so an apply sees one consistent state.
fn aliases_in(c: &Connection) -> Result<Aliases> {
    let mut s = c.prepare("SELECT pattern, canonical, is_regex FROM payee_alias ORDER BY id")?;
    let rows = s
        .query_map([], |r| {
            Ok(AliasRow { pattern: r.get(0)?, canonical: r.get(1)?, is_regex: r.get::<_, i64>(2)? != 0 })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Aliases::compile(&rows))
}

/// The `match_json` of a rule made from rows: their normalized payees, and an amount band when the rows agree.
/// `payees` must be sorted and unique and non-empty.
pub fn match_for(payees: &[String], amounts: &[f64]) -> Value {
    let mut m = Map::new();
    if payees.len() == 1 {
        m.insert("payee_norm".into(), json!(payees[0]));
    } else {
        m.insert("payee_norm_any".into(), json!(payees));
    }
    if let Some((lo, hi)) = tight_band(amounts) {
        m.insert("amount_min".into(), json!(lo));
        m.insert("amount_max".into(), json!(hi));
    }
    Value::Object(m)
}

/// `(min, max)` of the signed amounts, when they share a sign and every absolute value is within 10% of the
/// median absolute value.
fn tight_band(amounts: &[f64]) -> Option<(f64, f64)> {
    if amounts.is_empty() {
        return None;
    }
    let same_sign = amounts.iter().all(|&a| a >= 0.0) || amounts.iter().all(|&a| a < 0.0);
    if !same_sign {
        return None;
    }
    let mut abs: Vec<f64> = amounts.iter().map(|a| a.abs()).collect();
    abs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = abs.len();
    let med = if n % 2 == 1 { abs[n / 2] } else { (abs[n / 2 - 1] + abs[n / 2]) / 2.0 };
    if med <= 0.0 || abs.iter().any(|a| (a - med).abs() > 0.1 * med) {
        return None;
    }
    let lo = amounts.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = amounts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((lo, hi))
}

/// Id of an existing rule with the same match and category, if any.
fn existing_rule(c: &Connection, matched: &Value, category: i64) -> Result<Option<i64>> {
    let mut s = c.prepare("SELECT id, match_json FROM rule WHERE category_id=?1 ORDER BY id")?;
    let rows = s.query_map([category], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, m) = row?;
        if serde_json::from_str::<Value>(&m).ok().as_ref() == Some(matched) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

impl Db {
    /// Set the category of many rows, optionally with a rule for them. `make_rule`: `None` for no rule,
    /// `Some(name)` for a rule (the name is optional). Rows that already hold a different category are skipped
    /// unless `overwrite`. Unknown ids are counted as skipped. Nothing is written when the call changes nothing.
    pub fn smart_apply(
        &self,
        category: i64,
        ids: &[i64],
        overwrite: bool,
        make_rule: Option<Option<String>>,
    ) -> Result<SmartApplied, MutError> {
        if ids.is_empty() || ids.len() > MAX_APPLY_IDS {
            return Err(MutError::Invalid(format!("txn_ids must hold 1 to {MAX_APPLY_IDS} ids")));
        }
        let mut uniq = ids.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        let rule_name = match &make_rule {
            Some(Some(n)) if !n.trim().is_empty() => Some(categories::validate_name(n)?),
            _ => None,
        };

        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let exists: i64 = tx.query_row("SELECT count(*) FROM category WHERE id=?1", [category], |r| r.get(0))?;
        if exists == 0 {
            return Err(MutError::NotFound("category not found"));
        }
        let path = categories::path_of(&tx, category)?;
        let aliases = aliases_in(&tx)?;

        let mut p = Pending::default();
        let (mut changed, mut skipped, mut conflicts) = (0usize, 0usize, 0usize);
        // (normalized payee, signed amount) of every row that exists, for the rule.
        let mut seen: Vec<(String, f64)> = Vec::new();
        for id in uniq {
            let row: Option<(Option<String>, f64, Option<i64>)> = tx
                .query_row(
                    "SELECT payee, amount, category_local_id FROM transactions WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((payee, amount, cur)) = row else {
                skipped += 1;
                continue;
            };
            seen.push((normalize(payee.as_deref().unwrap_or(""), &aliases), amount));
            match cur {
                Some(c) if c == category => skipped += 1,
                Some(_) if !overwrite => conflicts += 1,
                _ => match categories::set_txn(&tx, &mut p, id, Some(category))? {
                    TxnSet::Changed => changed += 1,
                    TxnSet::Same | TxnSet::Missing => skipped += 1,
                },
            }
        }

        let mut rule_id = None;
        let mut new_rule = false;
        if make_rule.is_some() {
            if seen.is_empty() {
                return Err(MutError::Invalid("none of the ids exist, so there is no rule to make".into()));
            }
            let mut payees: Vec<String> = seen.iter().map(|(n, _)| n.clone()).collect();
            payees.sort();
            payees.dedup();
            let amounts: Vec<f64> = seen.iter().map(|(_, a)| *a).collect();
            let matched = match_for(&payees, &amounts);
            match existing_rule(&tx, &matched, category)? {
                Some(id) => rule_id = Some(id),
                None => {
                    let default_name = if payees.len() == 1 { payees[0].clone() } else { "smart rule".to_string() };
                    let rname = rule_name.clone().unwrap_or(default_name);
                    let position: i64 = tx.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM rule", [], |r| r.get(0))?;
                    tx.execute(
                        "INSERT INTO rule(position, name, enabled, match_json, category_id, hits) VALUES (?1,?2,1,?3,?4,0)",
                        params![position, rname, matched.to_string(), category],
                    )?;
                    let id = tx.last_insert_rowid();
                    let row = categories::rule_json(&tx, id)?.ok_or_else(|| anyhow::anyhow!("rule row vanished"))?;
                    p.push(Item::new("rule", id, "row", None, Some(row)));
                    rule_id = Some(id);
                    new_rule = true;
                }
            }
        }

        let label = match (changed, new_rule) {
            (0, false) => "No change".to_string(),
            (0, true) => format!("Create rule for {path}"),
            (n, rule) => {
                let plural = if n == 1 { "transaction" } else { "transactions" };
                format!("Categorize {n} {plural} as {path}{}", if rule { " (+rule)" } else { "" })
            }
        };
        let batch_id = p.commit(&tx, &label, "smart")?;
        tx.commit().map_err(MutError::from)?;
        Ok(SmartApplied { batch_id, label, changed, skipped, skipped_conflicts: conflicts, rule_id })
    }

    /// Compiled `payee_alias` rows, for normalizing payees outside a write (the preview).
    pub fn payee_aliases(&self) -> Result<Aliases> {
        let conn = self.lock()?;
        aliases_in(&conn)
    }

    /// Every rule, in the order the engine applies them.
    pub fn list_rules(&self) -> Result<Vec<RuleRow>> {
        let conn = self.lock()?;
        let mut s = conn.prepare(
            "SELECT id, position, name, enabled, match_json, category_id, hits FROM rule ORDER BY position, id",
        )?;
        let raw = s
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(raw.len());
        for (id, position, name, enabled, m, cat, hits) in raw {
            let category_path = match cat {
                Some(c) => Some(categories::path_of(&conn, c)?).filter(|p| !p.is_empty()),
                None => None,
            };
            out.push(RuleRow {
                id,
                position,
                name,
                enabled: enabled != 0,
                matched: serde_json::from_str(&m).unwrap_or_else(|_| json!({})),
                category_id: cat,
                category_path,
                hits,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
