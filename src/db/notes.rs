//! User notes on transactions (`transactions.user_notes`, migration v4). Journaled like every mutation:
//! one `change_batch` per call with a `transaction_notes` item per row that actually changed.

use super::journal::{Item, MutError, Outcome, Pending};
use super::Db;
use rusqlite::{params, OptionalExtension};
use serde_json::json;

/// Maximum note length in Unicode characters (the column is `CHAR(255)`).
pub const MAX_NOTE_CHARS: usize = 255;
/// At most this many transactions per call (same cap as categorize).
pub const MAX_NOTE_IDS: usize = super::categories::MAX_CATEGORY_IDS;

/// Trim, reject control characters and over-long text; empty becomes `None` (stored as NULL).
pub fn validate_note(raw: Option<&str>) -> Result<Option<String>, MutError> {
    let s = raw.unwrap_or("").trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.chars().count() > MAX_NOTE_CHARS {
        return Err(MutError::Invalid(format!("notes must be at most {MAX_NOTE_CHARS} characters")));
    }
    if s.chars().any(char::is_control) {
        return Err(MutError::Invalid("notes must not contain control characters".into()));
    }
    Ok(Some(s.to_string()))
}

impl Db {
    /// Set (or clear, with `None`/empty) the note of every listed transaction. An unknown id fails the
    /// whole call with 404 and changes nothing. Rows already holding that note are not journaled.
    pub fn set_notes(&self, ids: &[i64], notes: Option<&str>) -> Result<Outcome, MutError> {
        if ids.is_empty() {
            return Err(MutError::Invalid("ids must not be empty".into()));
        }
        if ids.len() > MAX_NOTE_IDS {
            return Err(MutError::Invalid(format!("at most {MAX_NOTE_IDS} transaction ids per call")));
        }
        let note = validate_note(notes)?;
        let mut uniq = ids.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let mut p = Pending::default();
        let mut changed = 0usize;
        for id in &uniq {
            let cur: Option<Option<String>> =
                tx.query_row("SELECT user_notes FROM transactions WHERE id=?1", [id], |r| r.get(0)).optional()?;
            let Some(cur) = cur else { return Err(MutError::NotFound("transaction not found")) };
            if cur == note {
                continue;
            }
            tx.execute("UPDATE transactions SET user_notes=?1 WHERE id=?2", params![note, id])?;
            p.push(Item::new("transaction_notes", *id, "user_notes", Some(json!(cur)), Some(json!(note))));
            changed += 1;
        }
        let plural = if changed == 1 { "transaction" } else { "transactions" };
        let label = match (changed, &note) {
            (0, _) => "No change".to_string(),
            (n, Some(_)) => format!("Set note on {n} {plural}"),
            (n, None) => format!("Clear note on {n} {plural}"),
        };
        let batch_id = p.commit(&tx, &label, "notes")?;
        tx.commit()?;
        Ok(Outcome { id: None, batch_id, label })
    }
}
