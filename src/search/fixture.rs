//! Small synthetic cache shared by the search tests. No real data.
//!
//! Categories: Home(1) > Improvement(2) > Paint(3); Home > Garden(4); Home > Zeta(7);
//! Salary(5) > Job(6). Accounts: 1 "Checking", 2 "Savings". `today()` is Thursday 2026-01-08.

use crate::cache::Cache;
use crate::db::{RawAccount, RawCategory, RawTxn, Snapshot};
use chrono::NaiveDate;

pub(crate) fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 8).unwrap()
}

fn cat(id: i64, parent_id: Option<i64>, name: &str, kind: &str, sort: i64) -> RawCategory {
    RawCategory { id, parent_id, name: name.into(), kind: kind.into(), sort }
}

#[allow(clippy::too_many_arguments)]
fn t(id: i64, date: &str, payee: &str, orig: Option<&str>, notes: Option<&str>, amount: f64, acct: Option<i64>, cat: Option<i64>, lm: Option<&str>) -> RawTxn {
    RawTxn {
        id,
        date: date.into(),
        payee: Some(payee.into()),
        original_name: orig.map(Into::into),
        notes: notes.map(Into::into),
        amount,
        currency: Some("USD".into()),
        category_local_id: cat,
        lm_category: lm.map(Into::into),
        account_id: acct,
        account: acct.map(|a| if a == 1 { "Checking".to_string() } else { "Savings".to_string() }),
        ..Default::default()
    }
}

pub(crate) fn fixture() -> Cache {
    let mut txns = vec![
        t(1, "2026-01-05", "Amazon", Some("AMZN MKTP"), Some("books"), 10.0, Some(1), Some(3), Some("Shopping")),
        t(2, "2026-01-03", "Whole Foods", Some("WF#123"), None, 50.0, Some(2), Some(4), Some("Groceries")),
        t(3, "2026-01-05", "Payroll", Some("ACH DEP"), Some("salary"), -1000.0, Some(1), Some(6), Some("Paycheck")),
        t(4, "2026-02-01", "Shell", None, None, 30.0, Some(2), None, None),
        t(5, "2025-12-31", "Coffee", None, None, 4.5, None, Some(7), Some("Coffee Shops")),
        t(6, "2026-01-05", "amazon", Some("AMZN REFUND"), None, -20.0, Some(1), Some(3), Some("Shopping")),
        t(7, "2026-01-07", "Rent", None, None, 1200.0, Some(1), Some(2), Some("Housing")),
        t(8, "not-a-date", "Broken", None, None, 1.0, Some(2), None, Some("Transfer to savings")),
        t(9, "2026-01-03", "Zero", None, None, 0.0, Some(2), Some(1), Some("Credit Card TRANSFER")),
    ];
    txns[3].is_pending = true;
    txns[6].recurring_id = Some(1);
    let accounts = vec![
        RawAccount { id: 1, name: "Chk raw".into(), display_name: Some("Checking".into()), ..Default::default() },
        RawAccount { id: 2, name: "Savings".into(), display_name: None, ..Default::default() },
    ];
    let categories = vec![
        cat(1, None, "Home", "expense", 1),
        cat(2, Some(1), "Improvement", "expense", 1),
        cat(3, Some(2), "Paint", "expense", 0),
        cat(4, Some(1), "Garden", "expense", 0),
        cat(5, None, "Salary", "income", 2),
        cat(6, Some(5), "Job", "income", 0),
        cat(7, Some(1), "Zeta", "expense", 0),
    ];
    Cache::build(Snapshot { txns, accounts, categories, balances: vec![] })
}
