use super::*;
use crate::db::Db;
use rusqlite::{params, Connection};
use serde_json::Value;

/// A migrated synthetic database, loaded into a state that has a cache.
/// Local categories: txn 1 -> Paint (Home › Improvement › Paint); the rest stay uncategorized.
fn loaded_state() -> (std::path::PathBuf, std::path::PathBuf, AppState) {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    drop(db);
    let c = raw(&path);
    c.execute_batch("UPDATE transactions SET category_local_id=(SELECT id FROM category WHERE name='Paint') WHERE id=1;").unwrap();
    drop(c);
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    let state = AppState::new(Settings::default(), Ok(db), "test");
    assert_eq!(state.refresh().unwrap(), 4);
    (dir, path, state)
}

/// A second connection, used only to write to the synthetic file behind the app's back.
fn raw(path: &std::path::Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.pragma_update(None, "key", crate::db::tests::PASS).unwrap();
    c
}

fn get(state: &AppState, path: &str) -> (u16, Value) {
    handle(state, "GET", path, "")
}

fn get_q(state: &AppState, path: &str, query: &str) -> (u16, Value) {
    handle(state, "GET", path, query)
}

fn ids(body: &Value) -> Vec<i64> {
    body["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect()
}

fn no_db() -> AppState {
    AppState::new(Settings::default(), Err("db missing".to_string()), "test")
}

// ---- routing without data ---------------------------------------------------------------------

#[test]
fn health_status_and_settings_are_unchanged() {
    let s = no_db();
    let (code, body) = get(&s, "/health");
    assert_eq!((code, body["ok"].as_bool()), (200, Some(true)));
    let (code, body) = get(&s, "/status");
    assert_eq!(code, 200);
    assert_eq!(body["db"]["state"], "error");
    let (code, _) = get(&s, "/settings");
    assert_eq!(code, 200);
}

#[test]
fn data_routes_without_database_are_503() {
    let s = no_db();
    for p in ["/transactions", "/transactions/1", "/accounts", "/categories"] {
        let (code, body) = get(&s, p);
        assert_eq!(code, 503, "{}", p);
        assert!(body["error"].is_string(), "{}", p);
    }
    // a bad parameter is refused before the database is consulted
    assert_eq!(get_q(&s, "/transactions", "limit=5000").0, 400);
    let (code, body) = handle(&s, "POST", "/refresh", "");
    assert_eq!(code, 503);
    assert_eq!(body["error"], "database unavailable");
}

#[test]
fn unknown_paths_are_404_for_any_method() {
    let s = no_db();
    for (m, p) in [
        ("GET", "/nope"),
        ("GET", "/transactions/"),
        ("GET", "/transactions/1/x"),
        ("GET", "/transactions/1/"),
        ("GET", "/accounts/1"),
        ("GET", "/categories/"),
        ("GET", "/refresh/"),
        ("GET", "/transactionsx"),
        ("GET", "/transaction"),
        ("POST", "/nope"),
        ("POST", "/refresh/"),
        ("GET", ""),
        ("GET", "/"),
    ] {
        let (code, body) = handle(&s, m, p, "");
        assert_eq!(code, 404, "{} {}", m, p);
        assert_eq!(body["error"], "not found", "{} {}", m, p);
    }
}

#[test]
fn known_paths_with_wrong_method_are_405() {
    let s = no_db();
    for (m, p) in [
        ("POST", "/transactions"),
        ("POST", "/transactions/1"),
        ("PUT", "/transactions/1"),
        ("DELETE", "/accounts"),
        ("POST", "/accounts"),
        ("POST", "/categories"),
        ("POST", "/status"),
        ("POST", "/health"),
        ("GET", "/refresh"),
        ("PUT", "/refresh"),
    ] {
        assert_eq!(handle(&s, m, p, "").0, 405, "{} {}", m, p);
    }
}

#[test]
fn route_names_for_logs() {
    assert_eq!(route_name("/transactions/42"), "transaction");
    assert_eq!(route_name("/transactions"), "transactions");
    assert_eq!(route_name("/refresh"), "refresh");
    assert_eq!(route_name("/nope/1"), "unknown");
}

// ---- with a loaded database -------------------------------------------------------------------

#[test]
fn transactions_default_shape_and_totals() {
    let (dir, _, s) = loaded_state();
    let (code, body) = get(&s, "/transactions");
    assert_eq!(code, 200);
    assert_eq!((body["all"].as_u64(), body["total"].as_u64()), (Some(4), Some(4)));
    assert_eq!(ids(&body), vec![3, 2, 1, 4]); // date desc, id desc on the 2026-01-03 tie
    assert_eq!(body["sum_out"], 12.0);
    assert_eq!(body["sum_in"], 0.0);
    let row = &body["rows"][0];
    let mut keys: Vec<&str> = row.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["account", "account_id", "amount", "category", "category_id", "currency", "date", "id", "lm_category", "payee", "pending", "recurring"]
    );
    // payee NULL shows as empty string; no account, no category
    assert_eq!(row["payee"], "");
    assert!(row["account"].is_null());
    assert!(row["category"].is_null());
    // txn 1 has a local category and a LM category
    let t1 = &body["rows"][2];
    assert_eq!(t1["id"], 1);
    assert_eq!(t1["category"], "Home › Improvement › Paint");
    assert_eq!(t1["category_id"].as_i64().is_some(), true);
    assert_eq!(t1["lm_category"], "LM Groceries");
    assert_eq!(t1["account"], "Checking");
    assert_eq!(t1["pending"], false);
    assert_eq!(t1["recurring"], false);
    // txn 2 uses manual_account_id
    assert_eq!(body["rows"][1]["account"], "Manual Cash");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transactions_filters_and_sorts_through_the_api() {
    let (dir, _, s) = loaded_state();
    let paint = {
        let (_, t1) = get(&s, "/transactions/1");
        t1["category_id"].as_i64().unwrap()
    };
    let (_, body) = get_q(&s, "/transactions", &format!("category={}", paint));
    assert_eq!((body["total"].as_u64(), ids(&body)), (Some(1), vec![1]));
    // category subtree: Home contains Paint
    let home = {
        let (_, cats) = get(&s, "/categories");
        cats["rows"].as_array().unwrap().iter().find(|r| r["name"] == "Home").unwrap()["id"].as_i64().unwrap()
    };
    assert_eq!(ids(&get_q(&s, "/transactions", &format!("category={}", home)).1), vec![1]);
    assert_eq!(ids(&get_q(&s, "/transactions", "category=none").1), vec![3, 2, 4]);
    assert_eq!(ids(&get_q(&s, "/transactions", "account=1").1), vec![1, 4]);
    assert_eq!(get_q(&s, "/transactions", "flow=in").1["total"], 0);
    let (_, b) = get_q(&s, "/transactions", "sort=amount&dir=asc&limit=2&offset=1");
    assert_eq!((b["total"].as_u64(), ids(&b)), (Some(4), vec![2, 3]));
    assert_eq!(b["sum_out"], 12.0); // sums cover all matches
    let (_, b) = get_q(&s, "/transactions", "q=uncategorised%20nothing");
    assert_eq!(b["total"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transactions_bad_parameters_are_400_with_an_error() {
    let (dir, _, s) = loaded_state();
    for qs in [
        "sort=bogus",
        "dir=up",
        "limit=0",
        "limit=501",
        "limit=abc",
        "offset=-1",
        "from=2026-02-30",
        "min=5.",
        "flow=both",
        "unknown=1",
        "limit=1&limit=2",
        "q=%ZZ",
        "from=2026-02-01&to=2026-01-01",
    ] {
        let (code, body) = get_q(&s, "/transactions", qs);
        assert_eq!(code, 400, "{}", qs);
        assert!(body["error"].as_str().is_some_and(|m| !m.is_empty()), "{}", qs);
    }
    // the value of a bad parameter is never echoed back
    let (_, body) = get_q(&s, "/transactions", "sort=SECRETVALUE");
    assert!(!body.to_string().contains("SECRETVALUE"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transaction_detail() {
    let (dir, _, s) = loaded_state();
    let (code, t3) = get(&s, "/transactions/3");
    assert_eq!(code, 200);
    assert_eq!(t3["id"], 3);
    assert_eq!(t3["payee"], "");
    assert_eq!(t3["tags"], serde_json::json!([]));
    assert_eq!(t3["recurring"], false);
    assert!(t3["original_name"].is_null());
    assert!(t3["notes"].is_null());
    assert!(t3["created_at_lm"].is_null());
    for k in ["status", "source", "recurring_id", "external_id", "updated_at_lm"] {
        assert!(t3.get(k).is_some(), "missing {}", k);
    }
    let (code, t1) = get(&s, "/transactions/1");
    assert_eq!(code, 200);
    assert_eq!(t1["category"], "Home › Improvement › Paint");
    assert_eq!(get(&s, "/transactions/999").0, 404);
    assert_eq!(get(&s, "/transactions/-1").0, 404);
    assert_eq!(get(&s, "/transactions/abc").0, 400);
    assert_eq!(get(&s, "/transactions/+1").0, 400);
    assert_eq!(get(&s, "/transactions/1.5").0, 400);
    assert_eq!(get(&s, "/transactions/%31").0, 400);
    assert_eq!(get(&s, "/transactions/").0, 404);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn accounts_route() {
    let (dir, _, s) = loaded_state();
    let (code, body) = get(&s, "/accounts");
    assert_eq!(code, 200);
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "Checking");
    assert_eq!((rows[0]["id"].as_i64(), rows[0]["count"].as_u64()), (Some(1), Some(2)));
    assert_eq!(rows[0]["sum_out"], 6.0); // txns 1 and 4
    assert!(rows[0]["balance"].is_null());
    assert_eq!(rows[1]["name"], "Manual Cash");
    assert_eq!((rows[1]["count"].as_u64(), rows[1]["sum_out"].as_f64()), (Some(1), Some(2.5)));
    for k in ["id", "name", "type", "institution", "mask", "balance", "currency", "count", "sum_out"] {
        assert!(rows[0].get(k).is_some(), "missing {}", k);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn categories_route() {
    let (dir, _, s) = loaded_state();
    let (code, body) = get(&s, "/categories");
    assert_eq!(code, 200);
    let rows = body["rows"].as_array().unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Home", "Improvement", "Paint", "Salary", "Job"]);
    let paint = &rows[2];
    assert_eq!((paint["depth"].as_u64(), paint["count"].as_u64(), paint["count_subtree"].as_u64()), (Some(2), Some(1), Some(1)));
    assert_eq!(paint["sum_out_subtree"], 1.5);
    let home = &rows[0];
    assert_eq!((home["parent_id"].is_null(), home["count_subtree"].as_u64()), (true, Some(1)));
    assert_eq!(rows[1]["parent_id"], home["id"]);
    assert_eq!(body["uncategorized"], 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refresh_picks_up_rows_inserted_after_startup() {
    let (dir, path, s) = loaded_state();
    // a sync writes a new row while the cache is in memory
    raw(&path)
        .execute(
            "INSERT INTO transactions(id,date,payee,amount,currency,plaid_account_id) VALUES (?1,?2,?3,?4,?5,?6)",
            params![5, "2026-03-01", "New", 9.25, "USD", 1],
        )
        .unwrap();
    assert_eq!(get(&s, "/transactions").1["all"], 4, "cache must not change until refreshed");
    assert_eq!(get_q(&s, "/transactions", "q=new").1["total"], 0);

    let (code, body) = handle(&s, "POST", "/refresh", "");
    assert_eq!(code, 200);
    assert_eq!(body, serde_json::json!({ "ok": true, "transactions": 5 }));

    let (_, all) = get(&s, "/transactions");
    assert_eq!((all["all"].as_u64(), all["total"].as_u64()), (Some(5), Some(5)));
    assert_eq!(all["rows"][0]["id"], 5);
    assert_eq!(get(&s, "/transactions/5").1["payee"], "New");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn get_refresh_is_405_and_the_cache_survives_a_failed_refresh() {
    let (dir, path, s) = loaded_state();
    assert_eq!(get(&s, "/refresh").0, 405);
    // a broken refresh must keep the old cache serving
    raw(&path).execute_batch("ALTER TABLE transactions RENAME TO transactions_gone;").unwrap();
    let (code, body) = handle(&s, "POST", "/refresh", "");
    assert_eq!(code, 500);
    assert_eq!(body["error"], "database query failed");
    assert_eq!(get(&s, "/transactions").1["all"], 4);
    let _ = std::fs::remove_dir_all(&dir);
}
