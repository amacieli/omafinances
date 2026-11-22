//! User nicknames on accounts (`accounts.nickname`, migration v5). Journaled like every mutation: one
//! `change_batch` per call with an `account_nickname` item when the value actually changed.

use super::journal::{Item, MutError, Outcome, Pending};
use super::Db;
use rusqlite::{params, OptionalExtension};
use serde_json::json;

/// Maximum nickname length in Unicode characters.
pub const MAX_NICKNAME_CHARS: usize = 40;

/// Trim, reject control characters and over-long text; empty or missing becomes `None` (stored as NULL).
pub fn validate_nickname(raw: Option<&str>) -> Result<Option<String>, MutError> {
    let s = raw.unwrap_or("").trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.chars().count() > MAX_NICKNAME_CHARS {
        return Err(MutError::Invalid(format!("nickname must be at most {MAX_NICKNAME_CHARS} characters")));
    }
    if s.chars().any(char::is_control) {
        return Err(MutError::Invalid("nickname must not contain control characters".into()));
    }
    Ok(Some(s.to_string()))
}

impl Db {
    /// Set (or clear, with `None`/empty) the nickname of one account. An unknown id is `NotFound` and
    /// changes nothing. Writing the value the account already has journals nothing.
    pub fn set_account_nickname(&self, account_id: i64, nickname: Option<&str>) -> Result<Outcome, MutError> {
        let nick = validate_nickname(nickname)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let cur: Option<Option<String>> = tx
            .query_row("SELECT nickname FROM accounts WHERE id=?1", [account_id], |r| r.get(0))
            .optional()?;
        let Some(cur) = cur else { return Err(MutError::NotFound("account not found")) };
        let mut p = Pending::default();
        let mut changed = false;
        if cur != nick {
            tx.execute("UPDATE accounts SET nickname=?1 WHERE id=?2", params![nick, account_id])?;
            p.push(Item::new("account_nickname", account_id, "nickname", Some(json!(cur)), Some(json!(nick))));
            changed = true;
        }
        let label = match (changed, &nick) {
            (false, _) => "No change".to_string(),
            (true, Some(_)) => "Set account nickname".to_string(),
            (true, None) => "Clear account nickname".to_string(),
        };
        let batch_id = p.commit(&tx, &label, "nickname")?;
        tx.commit()?;
        Ok(Outcome { id: None, batch_id, label })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::categories::tests::fresh;

    fn nick_of(db: &Db, id: i64) -> Option<String> {
        db.conn.lock().unwrap().query_row("SELECT nickname FROM accounts WHERE id=?1", [id], |r| r.get(0)).unwrap()
    }

    fn batches(db: &Db) -> i64 {
        db.conn.lock().unwrap().query_row("SELECT count(*) FROM change_batch", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn validate_trims_clears_and_limits() {
        assert_eq!(validate_nickname(Some("  Everyday  ")).unwrap().as_deref(), Some("Everyday"));
        assert_eq!(validate_nickname(Some("")).unwrap(), None);
        assert_eq!(validate_nickname(Some("   ")).unwrap(), None);
        assert_eq!(validate_nickname(None).unwrap(), None);
        // 40 characters is fine; multi-byte characters count once
        let ok = "é".repeat(MAX_NICKNAME_CHARS);
        assert_eq!(validate_nickname(Some(&ok)).unwrap().unwrap().chars().count(), 40);
        assert!(matches!(validate_nickname(Some(&"x".repeat(41))), Err(MutError::Invalid(_))));
        for bad in ["a\nb", "a\tb", "a\u{0}b", "a\u{7f}b"] {
            assert!(matches!(validate_nickname(Some(bad)), Err(MutError::Invalid(_))), "{bad:?}");
        }
    }

    #[test]
    fn set_clear_undo_redo() {
        let (_d, db) = fresh();
        assert_eq!(nick_of(&db, 1), None);
        let out = db.set_account_nickname(1, Some("  Everyday ")).unwrap();
        assert_eq!(out.label, "Set account nickname");
        assert!(out.batch_id.is_some());
        assert_eq!(nick_of(&db, 1).as_deref(), Some("Everyday"));

        db.set_account_nickname(1, Some("Second")).unwrap();
        db.undo().unwrap();
        assert_eq!(nick_of(&db, 1).as_deref(), Some("Everyday"), "undo restores the previous nickname");
        db.redo().unwrap();
        assert_eq!(nick_of(&db, 1).as_deref(), Some("Second"), "redo re-applies");
        db.undo().unwrap();
        db.undo().unwrap();
        assert_eq!(nick_of(&db, 1), None, "undo of the first set restores NULL");
        db.redo().unwrap();
        assert_eq!(nick_of(&db, 1).as_deref(), Some("Everyday"));

        let out = db.set_account_nickname(1, Some("   ")).unwrap();
        assert_eq!(out.label, "Clear account nickname");
        assert_eq!(nick_of(&db, 1), None, "empty clears to NULL, not an empty string");
        db.undo().unwrap();
        assert_eq!(nick_of(&db, 1).as_deref(), Some("Everyday"));
    }

    #[test]
    fn unchanged_value_journals_nothing_and_unknown_account_is_not_found() {
        let (_d, db) = fresh();
        db.set_account_nickname(2, Some("Pocket")).unwrap();
        let before = batches(&db);
        let out = db.set_account_nickname(2, Some("Pocket")).unwrap();
        assert_eq!((out.label.as_str(), out.batch_id), ("No change", None));
        let out = db.set_account_nickname(1, None).unwrap();
        assert_eq!(out.label, "No change");
        assert_eq!(batches(&db), before, "no batch for a no-op");

        assert!(matches!(db.set_account_nickname(99_999, Some("x")), Err(MutError::NotFound(_))));
        assert_eq!(batches(&db), before, "a failed call writes nothing");
        assert!(matches!(db.set_account_nickname(1, Some(&"x".repeat(41))), Err(MutError::Invalid(_))));
        assert_eq!(nick_of(&db, 1), None);
    }

    #[test]
    fn undo_refuses_when_the_row_changed_behind_the_journal() {
        let (_d, db) = fresh();
        db.set_account_nickname(1, Some("Mine")).unwrap();
        db.conn.lock().unwrap().execute("UPDATE accounts SET nickname='Other' WHERE id=1", []).unwrap();
        assert!(matches!(db.undo(), Err(MutError::Conflict(_))));
        assert_eq!(nick_of(&db, 1).as_deref(), Some("Other"), "a refused undo changes nothing");
    }
}
