use super::*;
use crate::db::Db;
use rusqlite::{params, Connection};
use serde_json::Value;

/// A migrated synthetic database, loaded into a state that has a cache.
/// Local categories: txn 1 -> Paint (Home › Improvement › Paint); the rest stay uncategorized.
fn loaded_state() -> (std::path::PathBuf, std::path::PathBuf, AppState) {
    loaded_state_with(Settings::default())
}

fn loaded_state_with(settings: Settings) -> (std::path::PathBuf, std::path::PathBuf, AppState) {
    let (dir, path) = crate::db::tests::setup();
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    db.migrate().unwrap();
    drop(db);
    let c = raw(&path);
    c.execute_batch("UPDATE transactions SET category_local_id=(SELECT id FROM category WHERE name='Paint') WHERE id=1;").unwrap();
    drop(c);
    let db = Db::open(&path, crate::db::tests::PASS).unwrap();
    let state = AppState::new(settings, Ok(db), "test");
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
    for p in ["/transactions", "/transactions/1", "/accounts", "/categories", "/rules"] {
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
        ("PUT", "/categories"),
        ("DELETE", "/categories/1/rename"),
        ("GET", "/undo"),
        ("GET", "/categories/1/rename"),
        ("GET", "/transactions/categorize"),
        ("POST", "/journal"),
        ("POST", "/status"),
        ("POST", "/health"),
        ("GET", "/refresh"),
        ("PUT", "/refresh"),
        ("GET", "/smart/preview"),
        ("GET", "/smart/rows"),
        ("PUT", "/smart/apply"),
        ("POST", "/rules"),
        ("DELETE", "/rules"),
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

// ---- slash search: /transactions?s=, /search/parse, /views, /history ----------------------------
//
// Synthetic data (see db::tests::setup): 1 "A" Checking Paint 1.5 (2026-01-01); 2 "B" Manual Cash
// 2.5 (01-03); 3 no payee 3.5 (01-03); 4 "D" Checking 4.5 (2025-12-31). "Today" is pinned to 2026-01-08.

fn slash_state() -> (std::path::PathBuf, std::path::PathBuf, AppState) {
    let (dir, path, s) = loaded_state();
    let s = s.with_config_dir(dir.join("cfg")).with_today(chrono::NaiveDate::from_ymd_opt(2026, 1, 8).unwrap());
    (dir, path, s)
}

fn send(state: &AppState, method: &str, path: &str, body: &str) -> (u16, Value) {
    handle_body(state, method, path, "", body.as_bytes())
}

fn s_ids(state: &AppState, qs: &str) -> Vec<i64> {
    let (code, body) = get_q(state, "/transactions", qs);
    assert_eq!(code, 200, "{} -> {}", qs, body);
    ids(&body)
}

#[test]
fn transactions_slash_search() {
    let (dir, _, s) = slash_state();
    assert_eq!(s_ids(&s, "s=a%3Acheck"), vec![1, 4]);
    assert_eq!(s_ids(&s, "s=-a%3Acheck"), vec![3, 2]);
    assert_eq!(s_ids(&s, "s=p%3Aa"), vec![1]);
    assert_eq!(s_ids(&s, "s=c%3Ahome"), vec![1]);
    assert_eq!(s_ids(&s, "s=c%3Anone"), vec![3, 2, 4]);
    assert_eq!(s_ids(&s, "s=lm%3Agroceries"), vec![1, 4]);
    assert_eq!(s_ids(&s, "s=d%3Athis-month"), vec![3, 2, 1]); // today pinned to 2026-01-08
    assert_eq!(s_ids(&s, "s=d%3A2025"), vec![4]);
    assert_eq!(s_ids(&s, "s=%3E3"), vec![3, 4]);
    assert_eq!(s_ids(&s, "s=a%3Acheck%20amount%3A%3E2"), vec![4]);

    // response is unchanged plus the canonical text; totals follow the search
    let (_, body) = get_q(&s, "/transactions", "s=A%3ACHECK%20%20amount%3A%3E1");
    assert_eq!(body["search"], serde_json::json!({ "canonical": "a:check amount:>1" }));
    assert_eq!((body["all"].as_u64(), body["total"].as_u64(), body["sum_out"].as_f64()), (Some(4), Some(2), Some(6.0)));
    let (_, plain) = get(&s, "/transactions");
    assert!(plain.get("search").is_none());
    let (_, empty) = get_q(&s, "/transactions", "s=");
    assert_eq!((empty["total"].as_u64(), empty["search"]["canonical"].as_str()), (Some(4), Some("")));

    // composes with the structured filters: final = structured AND search
    assert_eq!(s_ids(&s, "s=c%3Anone&account=1"), vec![4]);
    assert_eq!(s_ids(&s, "s=a%3Acheck&q=a"), vec![1]);
    assert_eq!(s_ids(&s, "s=amount%3A%3E1&flow=out&from=2026-01-02&to=2026-01-31"), vec![3, 2]);
    assert_eq!(s_ids(&s, "s=p%3Az"), Vec::<i64>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_sort_directive_applies_unless_the_request_sorts() {
    let (dir, _, s) = slash_state();
    assert_eq!(s_ids(&s, ""), vec![3, 2, 1, 4]);
    assert_eq!(s_ids(&s, "s=sort%3Aamount"), vec![1, 2, 3, 4]);
    assert_eq!(s_ids(&s, "s=sort%3A-amount"), vec![4, 3, 2, 1]);
    assert_eq!(s_ids(&s, "s=a%3Acheck%20sort%3Aamount"), vec![1, 4]);
    assert_eq!(s_ids(&s, "s=a%3Acheck%20sort%3A-amount"), vec![4, 1]);
    // explicit sort / dir win, each on its own
    assert_eq!(s_ids(&s, "s=a%3Acheck%20sort%3Aamount&sort=date&dir=asc"), vec![4, 1]);
    assert_eq!(s_ids(&s, "s=a%3Acheck%20sort%3Aamount&sort=date"), vec![1, 4]); // sort given: its default dir
    assert_eq!(s_ids(&s, "s=a%3Acheck%20sort%3A-amount&dir=asc"), vec![4, 1]); // dir given: the directive is ignored (date asc)
    // the directive is echoed in the canonical text
    let (_, body) = get_q(&s, "/transactions", "s=sort%3A-amount%20p%3Aa");
    assert_eq!(body["search"]["canonical"], "p:a sort:-amount");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transactions_search_errors() {
    let (dir, _, s) = slash_state();
    let (code, body) = get_q(&s, "/transactions", "s=pyee%3Ax");
    assert_eq!(code, 400);
    assert_eq!(body["error"], "parse");
    assert_eq!(body["parse_error"]["pos"], 0);
    assert_eq!(body["parse_error"]["len"], 4);
    assert_eq!(body["parse_error"]["message"], "unknown field \"pyee\"; did you mean \"p\"?");
    assert_eq!(body.as_object().unwrap().len(), 2);
    let (code, body) = get_q(&s, "/transactions", "s=a%20%22b%20c");
    assert_eq!((code, body["parse_error"]["pos"].as_u64(), body["parse_error"]["len"].as_u64()), (400, Some(2), Some(4)));
    for qs in ["s=a&s=b", "s=%zz", "s=%ff", "s=a&limit=5000", "s=a&sort=nope"] {
        let (code, body) = get_q(&s, "/transactions", qs);
        assert_eq!(code, 400, "{}", qs);
        assert!(body["error"].is_string());
        assert!(body.get("parse_error").is_none(), "{}", qs);
    }
    // the text of a failed query is never in the error body outside the parse message
    let (_, body) = get_q(&s, "/transactions", "s=zzsecretzz%3Ax");
    assert_eq!(body["error"], "parse");
    assert!(!body["error"].as_str().unwrap().contains("secret"));
    // without a database the search is still validated first, then 503
    assert_eq!(get_q(&no_db(), "/transactions", "s=pyee%3Ax").0, 400);
    assert_eq!(get_q(&no_db(), "/transactions", "s=p%3Ax").0, 503);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_parse_route() {
    let (dir, _, s) = slash_state();
    let (code, b) = get_q(&s, "/search/parse", "q=p%3Aa%20-is%3Apending");
    assert_eq!(code, 200);
    assert_eq!(b["ok"], true);
    assert_eq!(b["canonical"], "p:a -is:pending");
    assert_eq!(b["terms"][0], serde_json::json!({ "text": "p:a", "neg": false, "field": "payee", "value": "a" }));
    assert_eq!((b["terms"][1]["neg"].clone(), b["terms"][1]["field"].clone(), b["terms"][1]["value"].clone()), (true.into(), "is".into(), "pending".into()));
    assert!(b["sort"].is_null() && b["parse_error"].is_null());
    assert_eq!(b["matched"], 1);
    assert!(b["completions"]["items"].is_array());
    assert_eq!(b["completions"]["start"], 4);

    // free text, sort and an empty query
    let (_, b) = get_q(&s, "/search/parse", "q=hello%20sort%3A-amount");
    assert_eq!((b["terms"][0]["field"].as_str(), b["terms"][0]["value"].as_str(), b["sort"].as_str()), (Some("text"), Some("hello"), Some("-amount")));
    let (_, b) = get_q(&s, "/search/parse", "q=");
    assert_eq!((b["ok"].clone(), b["terms"].as_array().unwrap().len(), b["matched"].clone()), (true.into(), 0, 4.into()));
    let (_, b) = get(&s, "/search/parse");
    assert_eq!(b["ok"], true);

    // errors are 200 with ok:false and still carry completions
    let (code, b) = get_q(&s, "/search/parse", "q=p%3Aa%20pyee%3Ax");
    assert_eq!(code, 200);
    assert_eq!((b["ok"].clone(), b["canonical"].is_null(), b["matched"].is_null(), b["terms"].as_array().unwrap().len()), (false.into(), true, true, 0));
    assert_eq!((b["parse_error"]["pos"].as_u64(), b["parse_error"]["len"].as_u64()), (Some(4), Some(4)));
    assert!(b["parse_error"]["message"].as_str().unwrap().contains("did you mean \"p\""));
    assert!(b["completions"]["items"].is_array());

    // completions: field names, payees, accounts, categories, cursor
    let labels = |b: &Value| -> Vec<String> { b["completions"]["items"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect() };
    let (_, b) = get_q(&s, "/search/parse", "q=a");
    assert_eq!(labels(&b), vec!["a:", "amount:"]);
    assert_eq!((b["completions"]["start"].as_u64(), b["completions"]["end"].as_u64()), (Some(0), Some(1)));
    assert_eq!(b["completions"]["items"][0]["insert"], "a:");
    let (_, b) = get_q(&s, "/search/parse", "q=a%3A");
    assert_eq!(labels(&b), vec!["Checking", "Manual Cash"]);
    assert_eq!(b["completions"]["items"][0]["detail"], "2 txns");
    assert_eq!(b["completions"]["items"][1]["insert"], "a:\"Manual Cash\"");
    let (_, b) = get_q(&s, "/search/parse", "q=p%3A");
    assert!(labels(&b).contains(&"A".to_string()));
    let (_, b) = get_q(&s, "/search/parse", "q=c%3A");
    assert!(labels(&b).contains(&"Home".to_string()) && labels(&b).contains(&"none".to_string()));
    let (_, b) = get_q(&s, "/search/parse", "q=p%3Aa%20c%3Ah&pos=3");
    assert_eq!((b["completions"]["start"].as_u64(), b["completions"]["end"].as_u64()), (Some(0), Some(3)));
    let (_, b) = get_q(&s, "/search/parse", "q=p%3Aa%20c%3Ah&pos=999");
    assert_eq!(b["completions"]["start"], 4);
    let (_, b) = get_q(&s, "/search/parse", "q=%C3%A9&pos=1"); // inside a multibyte char: no panic
    assert_eq!(b["ok"], true);

    // bad parameters
    for qs in ["q=a&pos=x", "q=a&pos=-1", "q=a&pos=%2B1", "q=a&x=1", "q=a&q=b", "q=%zz", "pos=1&pos=2"] {
        assert_eq!(get_q(&s, "/search/parse", qs).0, 400, "{}", qs);
    }
    // no database: still parses, matched is null
    let (code, b) = get_q(&no_db(), "/search/parse", "q=p%3Aa");
    assert_eq!((code, b["ok"].clone(), b["matched"].is_null()), (200, true.into(), true));
    let _ = std::fs::remove_dir_all(&dir);
}

fn journal_rows(path: &std::path::Path) -> Vec<(String, String, String, Option<String>, Option<String>)> {
    let c = raw(path);
    let mut st = c
        .prepare("SELECT b.kind, b.label, i.entity, i.old_value, i.new_value FROM change_batch b JOIN change_item i ON i.batch_id=b.id ORDER BY b.id, i.id")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn views_crud_ordering_and_upsert() {
    let (dir, path, s) = slash_state();
    assert_eq!(get(&s, "/views"), (200, serde_json::json!({ "rows": [] })));

    let (code, v1) = send(&s, "POST", "/views", r#"{"name":"Big spends","query":"amount:>50"}"#);
    assert_eq!(code, 200);
    assert_eq!((v1["name"].clone(), v1["query"].clone(), v1["sort"].is_null(), v1["position"].clone()), ("Big spends".into(), "amount:>50".into(), true, 1.into()));
    let id1 = v1["id"].as_i64().unwrap();
    let mut keys: Vec<&str> = v1.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, vec!["id", "name", "position", "query", "sort"]);
    let (_, v2) = send(&s, "POST", "/views", r#"{"name":"Amazon","query":"p:amazon d:this-year","sort":"-amount"}"#);
    assert_eq!((v2["sort"].clone(), v2["position"].clone()), ("-amount".into(), 2.into()));

    let rows = |s: &AppState| -> Vec<String> { get(s, "/views").1["rows"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect() };
    assert_eq!(rows(&s), vec!["Big spends", "Amazon"]); // by position, then name

    // upsert by name, case-insensitively: same id and position, new spelling and query
    let (code, up) = send(&s, "POST", "/views", r#"{"name":"BIG SPENDS","query":"amount:>100","sort":"date"}"#);
    assert_eq!(code, 200);
    assert_eq!((up["id"].as_i64(), up["position"].as_i64(), up["name"].as_str(), up["query"].as_str(), up["sort"].as_str()), (Some(id1), Some(1), Some("BIG SPENDS"), Some("amount:>100"), Some("date")));
    assert_eq!(rows(&s), vec!["BIG SPENDS", "Amazon"]);
    // sort omitted on upsert clears it; name and query are trimmed
    let (_, up) = send(&s, "POST", "/views", "{\"name\":\"  big spends \",\"query\":\" amount:>100 \"}");
    assert_eq!((up["name"].as_str(), up["query"].as_str(), up["sort"].is_null()), (Some("big spends"), Some("amount:>100"), true));
    // bound parameters: odd characters are data
    let nasty = "x'); DROP TABLE saved_view;--";
    let (code, v) = send(&s, "POST", "/views", &serde_json::json!({ "name": nasty, "query": "\"it's\"" }).to_string());
    assert_eq!(code, 200);
    assert_eq!(get(&s, "/views").1["rows"].as_array().unwrap().len(), 3);
    assert_eq!(get(&s, "/views").1["rows"][2]["name"], nasty);
    // name of exactly 60 characters (multibyte counts as characters)
    assert_eq!(send(&s, "POST", "/views", &serde_json::json!({ "name": "é".repeat(60), "query": "a" }).to_string()).0, 200);

    // delete
    let (code, b) = send(&s, "DELETE", &format!("/views/{}", v["id"]), "");
    assert_eq!((code, b), (200, serde_json::json!({ "ok": true })));
    assert_eq!(send(&s, "DELETE", &format!("/views/{}", v["id"]), "").0, 404);
    assert_eq!(send(&s, "DELETE", "/views/99999", "").0, 404);
    assert_eq!(send(&s, "DELETE", "/views/abc", "").0, 400);
    assert_eq!(send(&s, "DELETE", "/views/1.5", "").0, 400);
    assert_eq!(send(&s, "DELETE", "/views/+1", "").0, 400);
    assert_eq!(send(&s, "DELETE", "/views/-1", "").0, 404);
    assert_eq!(rows(&s).len(), 3);

    // journal: one batch (kind view) with one item per write, none for refused ones
    let j = journal_rows(&path);
    let kinds: Vec<(&str, &str, &str)> = j.iter().map(|r| (r.0.as_str(), r.1.as_str(), r.2.as_str())).collect();
    assert_eq!(
        kinds,
        vec![
            ("view", "add view", "saved_view"),
            ("view", "add view", "saved_view"),
            ("view", "update view", "saved_view"),
            ("view", "update view", "saved_view"),
            ("view", "add view", "saved_view"),
            ("view", "add view", "saved_view"),
            ("view", "delete view", "saved_view"),
        ]
    );
    assert!(j[0].3.is_none() && j[0].4.as_deref().unwrap().contains("amount:>50"));
    assert!(j[2].3.as_deref().unwrap().contains("amount:>50") && j[2].4.as_deref().unwrap().contains("amount:>100"));
    assert!(j[6].3.as_deref().unwrap().contains("DROP TABLE") && j[6].4.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_validation() {
    let (dir, path, s) = slash_state();
    let bad = |body: &str| send(&s, "POST", "/views", body);
    for body in [
        r#"{"name":"","query":"a"}"#,
        r#"{"name":"   ","query":"a"}"#,
        r#"{"query":"a"}"#,
        r#"{"name":null,"query":"a"}"#,
        r#"{"name":5,"query":"a"}"#,
        r#"{"name":"x"}"#,
        r#"{"name":"x","query":7}"#,
        r#"{"name":"x","query":"a","sort":"nope"}"#,
        r#"{"name":"x","query":"a","sort":"-"}"#,
        r#"{"name":"x","query":"a","sort":3}"#,
        "{\"name\":\"a\\nb\",\"query\":\"a\"}",
        "{\"name\":\"a\\u0007b\",\"query\":\"a\"}",
        "[]",
        "\"x\"",
        "5",
        "null",
        "",
        "not json",
        "{\"name\":\"x\",",
    ] {
        let (code, b) = bad(body);
        assert_eq!(code, 400, "{}", body);
        assert!(b["error"].is_string(), "{}", body);
    }
    assert_eq!(bad(&serde_json::json!({ "name": "n".repeat(61), "query": "a" }).to_string()).0, 400);
    assert_eq!(bad(&serde_json::json!({ "name": "é".repeat(61), "query": "a" }).to_string()).0, 400);
    assert_eq!(bad(&serde_json::json!({ "name": "n", "query": "a".repeat(501) }).to_string()).0, 400);
    // unparsable query: the parse error, with position
    let (code, b) = bad(r#"{"name":"x","query":"p:a pyee:b"}"#);
    assert_eq!((code, b["error"].clone(), b["parse_error"]["pos"].clone()), (400, "parse".into(), 4.into()));
    let (code, b) = bad(r#"{"name":"x","query":"\"open"}"#);
    assert_eq!((code, b["error"].clone()), (400, "parse".into()));
    // nothing was stored or journaled by any refused write
    assert_eq!(get(&s, "/views").1["rows"].as_array().unwrap().len(), 0);
    assert!(journal_rows(&path).is_empty());
    // the empty query is a valid (match-everything) view; a sort key may carry a minus
    assert_eq!(bad(r#"{"name":"all","query":"","sort":"-payee"}"#).0, 200);
    assert_eq!(bad(r#"{"name":"empty sort","query":"a","sort":""}"#).1["sort"], Value::Null);
    // Content-Type is not needed and a body with surrounding whitespace is fine
    assert_eq!(bad(" \n{\"name\":\"ws\",\"query\":\"a\"}\n ").0, 200);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_and_history_without_a_database_or_config_dir() {
    let s = no_db();
    assert_eq!(send(&s, "GET", "/views", "").0, 503);
    assert_eq!(send(&s, "POST", "/views", r#"{"name":"x","query":"a"}"#).0, 503);
    assert_eq!(send(&s, "DELETE", "/views/1", "").0, 503);
    assert_eq!(send(&s, "GET", "/history", "").0, 503);
    assert_eq!(send(&s, "POST", "/history", r#"{"q":"a"}"#).0, 503);
}

#[test]
fn history_dedupe_cap_and_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _, s) = slash_state();
    let file = dir.join("cfg").join("search_history.json");
    assert_eq!(send(&s, "GET", "/history", ""), (200, serde_json::json!({ "rows": [] })));
    let rows = |s: &AppState| -> Vec<String> { send(s, "GET", "/history", "").1["rows"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };

    assert_eq!(send(&s, "POST", "/history", r#"{"q":"p:a"}"#), (200, serde_json::json!({ "ok": true, "added": true })));
    send(&s, "POST", "/history", r#"{"q":"a:check"}"#);
    assert_eq!(rows(&s), vec!["a:check", "p:a"]); // newest first
    send(&s, "POST", "/history", r#"{"q":"p:a"}"#);
    assert_eq!(rows(&s), vec!["p:a", "a:check"]); // the older identical entry moved, not duplicated
    // text is stored as typed (trimmed), not canonicalised
    send(&s, "POST", "/history", "{\"q\":\"  P:A  amount:>1 \"}");
    assert_eq!(rows(&s)[0], "P:A  amount:>1");

    // empty and unparsable queries are ignored, not errors
    for q in ["", "   ", "pyee:x", "\"open"] {
        let (code, b) = send(&s, "POST", "/history", &serde_json::json!({ "q": q }).to_string());
        assert_eq!((code, b["added"].clone()), (200, false.into()), "{:?}", q);
    }
    assert_eq!(rows(&s).len(), 3);
    // too long is a 400; exactly 500 characters is accepted
    assert_eq!(send(&s, "POST", "/history", &serde_json::json!({ "q": "a".repeat(501) }).to_string()).0, 400);
    assert_eq!(send(&s, "POST", "/history", &serde_json::json!({ "q": "é".repeat(501) }).to_string()).0, 400);
    assert_eq!(send(&s, "POST", "/history", &serde_json::json!({ "q": "a".repeat(500) }).to_string()).0, 200);
    assert_eq!(rows(&s)[0].len(), 500);
    // body errors
    for body in ["", "x", "[]", "{}", r#"{"q":5}"#, r#"{"q":null}"#, r#"{"query":"a"}"#] {
        assert_eq!(send(&s, "POST", "/history", body).0, 400, "{}", body);
    }

    // file: 0600 in a private dir, atomic (no temp files left), holds query text only
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let leftovers: Vec<_> = std::fs::read_dir(dir.join("cfg")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(leftovers, vec!["search_history.json"]);
    let text = std::fs::read_to_string(&file).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(v.as_array().unwrap().iter().all(Value::is_string));

    // cap at 50, newest first
    for i in 0..60 {
        assert_eq!(send(&s, "POST", "/history", &serde_json::json!({ "q": format!("p:x{}", i) }).to_string()).0, 200);
    }
    let r = rows(&s);
    assert_eq!(r.len(), 50);
    assert_eq!((r[0].as_str(), r[49].as_str()), ("p:x59", "p:x10"));
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);

    // a damaged file reads as empty and is replaced by the next add
    std::fs::write(&file, "{{ not json").unwrap();
    assert!(rows(&s).is_empty());
    send(&s, "POST", "/history", r#"{"q":"a"}"#);
    assert_eq!(rows(&s), vec!["a"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_routes_methods_and_paths() {
    let (dir, _, s) = slash_state();
    for (m, p, want) in [
        ("DELETE", "/views", 405),
        ("PUT", "/views", 405),
        ("GET", "/views/1", 405),
        ("POST", "/views/1", 405),
        ("PUT", "/views/1", 405),
        ("DELETE", "/history", 405),
        ("PUT", "/history", 405),
        ("POST", "/search/parse", 405),
        ("DELETE", "/search/parse", 405),
        ("DELETE", "/transactions", 405),
        ("DELETE", "/transactions/1", 405),
        ("DELETE", "/refresh", 405),
        ("DELETE", "/health", 405),
        ("DELETE", "/status", 405),
        ("PATCH", "/accounts", 405),
        ("GET", "/views/", 404),
        ("DELETE", "/views/", 404),
        ("GET", "/views/1/x", 404),
        ("DELETE", "/views/1/", 404),
        ("GET", "/history/", 404),
        ("GET", "/search", 404),
        ("GET", "/search/parse/", 404),
        ("GET", "/search/parsex", 404),
        ("GET", "/viewss", 404),
        ("GET", "/History", 404),
    ] {
        assert_eq!(send(&s, m, p, "").0, want, "{} {}", m, p);
    }
    assert_eq!(route_name("/views"), "views");
    assert_eq!(route_name("/views/12"), "view");
    assert_eq!(route_name("/history"), "history");
    assert_eq!(route_name("/search/parse"), "search_parse");
    // other routes ignore a body
    assert_eq!(send(&s, "GET", "/health", "{\"x\":1}").0, 200);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- category tree, categorize, undo/redo ---------------------------------------------------

fn cat_id(state: &AppState, name: &str) -> i64 {
    let (_, body) = get(state, "/categories");
    body["rows"].as_array().unwrap().iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("no {name}"))["id"]
        .as_i64()
        .unwrap()
}

#[test]
fn categories_rows_carry_kind_and_sort() {
    let (dir, _, s) = loaded_state();
    let (_, body) = get(&s, "/categories");
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows[0]["kind"], "expense");
    assert_eq!(rows[0]["sort"], 1); // seeded sort values, not renumbered
    assert_eq!(rows[3]["name"], "Salary");
    assert_eq!(rows[3]["kind"], "income");
    assert_eq!(rows[3]["sort"], 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn post_categories_creates_and_reports_errors() {
    let (dir, _, s) = loaded_state();
    let (code, body) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"  ApiTop  "}"#);
    assert_eq!(code, 200, "{body}");
    assert_eq!((body["ok"].as_bool(), body["label"].as_str()), (Some(true), Some("Create \"ApiTop\"")));
    assert!(body["id"].is_i64() && body["batch_id"].is_i64());
    // visible in the very next read
    let (_, cats) = get(&s, "/categories");
    assert!(cats["rows"].as_array().unwrap().iter().any(|r| r["name"] == "ApiTop"));

    let top = body["id"].as_i64().unwrap();
    let kid = send(&s, "POST", "/categories", &format!(r#"{{"parent_id":{top},"name":"ApiKid","kind":"income"}}"#));
    assert_eq!(kid.0, 200);
    let (_, cats) = get(&s, "/categories");
    let k = cats["rows"].as_array().unwrap().iter().find(|r| r["name"] == "ApiKid").unwrap();
    assert_eq!(k["kind"], "income");

    assert_eq!(send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"apitop"}"#).0, 409, "sibling clash");
    assert_eq!(send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"   "}"#).0, 400);
    assert_eq!(send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"x","kind":"savings"}"#).0, 400);
    assert_eq!(send(&s, "POST", "/categories", r#"{"parent_id":999999,"name":"x"}"#).0, 404);
    assert_eq!(send(&s, "POST", "/categories", r#"{"parent_id":"1","name":"x"}"#).0, 400);
    assert_eq!(send(&s, "POST", "/categories", r#"{"name":"x"}"#).0, 400, "parent_id is required");
    assert_eq!(send(&s, "POST", "/categories", r#"[1]"#).0, 400);
    assert_eq!(send(&s, "POST", "/categories", "{not json").0, 400);
    let (code, body) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"x"}"#);
    assert_eq!(code, 200, "{body}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn post_categories_error_envelope_codes() {
    let (dir, _, s) = loaded_state();
    let (_, b) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"Dup"}"#);
    assert_eq!(b["ok"], true);
    let (code, body) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"dup"}"#);
    assert_eq!((code, body["error"].as_str()), (409, Some("conflict")));
    assert!(body["message"].is_string());
    let (code, body) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":""}"#);
    assert_eq!((code, body["error"].as_str()), (400, Some("invalid")));
    let (code, body) = send(&s, "POST", "/categories", r#"{"parent_id":77777,"name":"q"}"#);
    assert_eq!((code, body["error"].as_str()), (404, Some("not_found")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn depth_limit_is_enforced_by_the_api_and_honours_settings() {
    let (dir, _, s) = loaded_state(); // default max_depth 3
    let l2 = cat_id(&s, "Improvement");
    let l3 = cat_id(&s, "Paint");
    let (code, body) = send(&s, "POST", "/categories", &format!(r#"{{"parent_id":{l3},"name":"TooDeep"}}"#));
    assert_eq!((code, body["error"].as_str()), (409, Some("limit")), "{body}");
    assert!(body["message"].as_str().unwrap().contains('3'));
    assert_eq!(send(&s, "POST", "/categories", &format!(r#"{{"parent_id":{l2},"name":"Fits"}}"#)).0, 200);
    let _ = std::fs::remove_dir_all(&dir);

    let mut st = Settings::default();
    st.categories.max_depth = 2;
    let (dir, _, s) = loaded_state_with(st);
    let home = cat_id(&s, "Home");
    let (code, body) = send(&s, "POST", "/categories", &format!(r#"{{"parent_id":{home},"name":"Lvl2"}}"#));
    assert_eq!(code, 200, "{body}");
    let lvl2 = body["id"].as_i64().unwrap();
    let (code, body) = send(&s, "POST", "/categories", &format!(r#"{{"parent_id":{lvl2},"name":"Lvl3"}}"#));
    assert_eq!((code, body["error"].as_str()), (409, Some("limit")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_reports_max_depth() {
    let s = no_db();
    let (code, body) = get(&s, "/settings");
    assert_eq!((code, body["max_depth"].as_u64()), (200, Some(3)));
    let mut st = Settings::default();
    st.categories.max_depth = 5;
    let s = AppState::new(st, Err("db missing".to_string()), "test");
    assert_eq!(get(&s, "/settings").1["max_depth"].as_u64(), Some(5));
}

#[test]
fn category_actions_rename_kind_move_shift_delete_merge() {
    let (dir, _, s) = loaded_state();
    let home = cat_id(&s, "Home");
    let improvement = cat_id(&s, "Improvement");
    let salary = cat_id(&s, "Salary");
    let job = cat_id(&s, "Job");

    let (code, body) = send(&s, "POST", &format!("/categories/{home}/rename"), r#"{"name":"House"}"#);
    assert_eq!((code, body["label"].as_str()), (200, Some("Rename \"Home\" to \"House\"")));
    assert!(body["batch_id"].is_i64());
    assert_eq!(send(&s, "POST", &format!("/categories/{home}/rename"), r#"{"name":"house"}"#).0, 200, "case-only rename of itself");
    assert_eq!(send(&s, "POST", &format!("/categories/{salary}/rename"), r#"{"name":"house"}"#).0, 409);

    let (code, body) = send(&s, "POST", &format!("/categories/{salary}/kind"), r#"{"kind":"transfer"}"#);
    assert_eq!((code, body["ok"].as_bool()), (200, Some(true)));
    assert_eq!(send(&s, "POST", &format!("/categories/{salary}/kind"), r#"{"kind":"nope"}"#).0, 400);

    // move Job to the top level, then shift it up and down
    let (code, _) = send(&s, "POST", &format!("/categories/{job}/move"), r#"{"parent_id":null,"position":0}"#);
    assert_eq!(code, 200);
    let (_, cats) = get(&s, "/categories");
    assert_eq!(cats["rows"][0]["name"], "Job");
    let (code, body) = send(&s, "POST", &format!("/categories/{job}/shift"), r#"{"delta":1}"#);
    assert_eq!((code, body["ok"].as_bool()), (200, Some(true)));
    let (_, cats) = get(&s, "/categories");
    assert_ne!(cats["rows"][0]["name"], "Job");
    assert_eq!(send(&s, "POST", &format!("/categories/{job}/shift"), r#"{"delta":2}"#).0, 400);
    assert_eq!(send(&s, "POST", &format!("/categories/{job}/move"), &format!(r#"{{"parent_id":{job}}}"#)).0, 409, "under itself");

    // merge Improvement into Salary (different subtrees, no clash), then delete the now-empty Job
    let (code, body) = send(&s, "POST", &format!("/categories/{improvement}/merge"), &format!(r#"{{"into":{salary}}}"#));
    assert_eq!(code, 200, "{body}");
    assert!(body["label"].as_str().unwrap().starts_with("Merge \"Improvement\" into"));
    assert_eq!(send(&s, "POST", &format!("/categories/{salary}/merge"), &format!(r#"{{"into":{salary}}}"#)).0, 409);

    let (code, body) = send(&s, "POST", &format!("/categories/{job}/delete"), r#"{"reassign_to":null}"#);
    assert_eq!((code, body["ok"].as_bool()), (200, Some(true)), "{body}");
    let (_, cats) = get(&s, "/categories");
    assert!(!cats["rows"].as_array().unwrap().iter().any(|r| r["name"] == "Job"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn category_action_errors_and_routing() {
    let (dir, _, s) = loaded_state();
    let paint = cat_id(&s, "Paint");
    assert_eq!(send(&s, "POST", "/categories/999999/rename", r#"{"name":"x"}"#).0, 404);
    assert_eq!(send(&s, "POST", "/categories/abc/rename", r#"{"name":"x"}"#).0, 400);
    assert_eq!(send(&s, "POST", &format!("/categories/{paint}/rename"), r#"{"name":""}"#).0, 400);
    assert_eq!(send(&s, "POST", &format!("/categories/{paint}/rename"), r#"{}"#).0, 400);
    assert_eq!(send(&s, "POST", &format!("/categories/{paint}/delete"), r#"{"reassign_to":999999}"#).0, 404);
    assert_eq!(send(&s, "POST", &format!("/categories/{paint}/merge"), &format!(r#"{{"into":{paint}}}"#)).0, 409);
    assert_eq!(send(&s, "POST", &format!("/categories/{paint}/shift"), r#"{"delta":"1"}"#).0, 400);
    // unknown action and wrong method
    assert_eq!(send(&s, "POST", &format!("/categories/{paint}/bogus"), "{}").0, 404);
    assert_eq!(get(&s, &format!("/categories/{paint}/rename")).0, 405);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn categorize_route_counts_and_validates() {
    let (dir, _, s) = loaded_state();
    let job = cat_id(&s, "Job");
    let paint = cat_id(&s, "Paint");
    let (code, body) = send(&s, "POST", "/transactions/categorize", &format!(r#"{{"ids":[1,2,99999],"category_id":{job}}}"#));
    assert_eq!(code, 200, "{body}");
    assert_eq!((body["changed"].as_u64(), body["skipped"].as_u64()), (Some(2), Some(1)));
    assert_eq!(body["label"], "Categorize 2 transactions as Salary › Job");
    assert!(body["batch_id"].is_i64());
    // already equal counts as skipped; uncategorize with null
    let (_, body) = send(&s, "POST", "/transactions/categorize", &format!(r#"{{"ids":[2],"category_id":{job}}}"#));
    assert_eq!((body["changed"].as_u64(), body["skipped"].as_u64(), body["batch_id"].is_null()), (Some(0), Some(1), true));
    let (_, body) = send(&s, "POST", "/transactions/categorize", r#"{"ids":[2],"category_id":null}"#);
    assert_eq!(body["changed"], 1);
    assert_eq!(send(&s, "POST", "/transactions/categorize", &format!(r#"{{"ids":[1],"category_id":{paint}}}"#)).0, 200);

    assert_eq!(send(&s, "POST", "/transactions/categorize", r#"{"ids":"1","category_id":null}"#).0, 400);
    assert_eq!(send(&s, "POST", "/transactions/categorize", r#"{"ids":[1,"x"],"category_id":null}"#).0, 400);
    assert_eq!(send(&s, "POST", "/transactions/categorize", r#"{"ids":[],"category_id":null}"#).0, 400);
    assert_eq!(send(&s, "POST", "/transactions/categorize", r#"{"ids":[1],"category_id":424242}"#).0, 404);
    let many: Vec<String> = (1..=5001).map(|i| i.to_string()).collect();
    assert_eq!(send(&s, "POST", "/transactions/categorize", &format!(r#"{{"ids":[{}],"category_id":null}}"#, many.join(","))).0, 400);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_redo_and_journal_routes() {
    let (dir, _, s) = loaded_state();
    let (code, body) = send(&s, "POST", "/undo", "");
    assert_eq!(code, 409, "{body}");
    assert_eq!(body["error"], "nothing_to_undo");
    assert_eq!(send(&s, "POST", "/redo", "").1["error"], "nothing_to_redo");

    let (_, made) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"UndoMe"}"#);
    let (_, j) = get_q(&s, "/journal", "");
    assert_eq!(j["can_undo"], true);
    assert_eq!(j["next_undo_label"], "Create \"UndoMe\"");
    assert_eq!(j["rows"].as_array().unwrap().len(), 1, "default limit 20, one batch so far");
    assert_eq!(j["rows"][0]["kind"], "category");
    assert_eq!((j["rows"][0]["undone"].as_bool(), j["rows"][0]["superseded"].as_bool()), (Some(false), Some(false)));

    let (code, body) = send(&s, "POST", "/undo", "");
    assert_eq!((code, body["ok"].as_bool(), body["label"].as_str()), (200, Some(true), Some("Create \"UndoMe\"")));
    assert!(body["batch_id"].is_i64());
    assert!(!get(&s, "/categories").1["rows"].as_array().unwrap().iter().any(|r| r["name"] == "UndoMe"));
    let (_, j) = get_q(&s, "/journal", "");
    assert_eq!((j["can_redo"].as_bool(), j["next_redo_label"].as_str()), (Some(true), Some("Create \"UndoMe\"")));

    let (code, body) = send(&s, "POST", "/redo", "");
    assert_eq!((code, body["ok"].as_bool()), (200, Some(true)));
    assert!(get(&s, "/categories").1["rows"].as_array().unwrap().iter().any(|r| r["name"] == "UndoMe"));
    let _ = made;

    // limits on the journal query
    assert_eq!(get_q(&s, "/journal", "limit=1").0, 200);
    assert_eq!(get_q(&s, "/journal", "limit=100").0, 200);
    for q in ["limit=0", "limit=101", "limit=abc", "limit=", "limit=-1", "limit=1&limit=2", "other=1", "limit=2&x=1"] {
        let (code, body) = get_q(&s, "/journal", q);
        assert_eq!((code, body["error"].as_str()), (400, Some("invalid")), "{q}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_reports_conflict_when_data_changed_underneath() {
    let (dir, path, s) = loaded_state();
    let (_, made) = send(&s, "POST", "/categories", r#"{"parent_id":null,"name":"Tampered"}"#);
    let id = made["id"].as_i64().unwrap();
    raw(&path)
        .execute("UPDATE category SET name='Changed Elsewhere' WHERE id=?1", params![id])
        .unwrap();
    let (code, body) = send(&s, "POST", "/undo", "");
    assert_eq!((code, body["error"].as_str()), (409, Some("conflict")), "{body}");
    assert!(body["message"].is_string());
    let (_, j) = get_q(&s, "/journal", "");
    assert_eq!(j["can_undo"], true, "the refused batch stays undoable state-wise");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_routes_without_a_database_are_503() {
    let s = no_db();
    for (m, p, b) in [
        ("POST", "/categories", r#"{"parent_id":null,"name":"x"}"#),
        ("POST", "/categories/1/rename", r#"{"name":"x"}"#),
        ("POST", "/categories/1/kind", r#"{"kind":"income"}"#),
        ("POST", "/categories/1/move", r#"{"parent_id":null}"#),
        ("POST", "/categories/1/shift", r#"{"delta":1}"#),
        ("POST", "/categories/1/delete", r#"{"reassign_to":null}"#),
        ("POST", "/categories/1/merge", r#"{"into":2}"#),
        ("POST", "/transactions/categorize", r#"{"ids":[1],"category_id":null}"#),
        ("POST", "/undo", ""),
        ("POST", "/redo", ""),
        ("POST", "/smart/preview", r#"{"ids":[1],"category_id":1}"#),
        ("POST", "/smart/rows", r#"{"ids":[1]}"#),
        ("POST", "/smart/apply", r#"{"category_id":1,"txn_ids":[1]}"#),
    ] {
        let (code, body) = send(&s, m, p, b);
        assert_eq!(code, 503, "{m} {p}");
        assert!(body["error"].is_string(), "{m} {p}");
    }
    assert_eq!(get(&s, "/journal").0, 503);
}

// ---- categorize with preview (smart routes) ---------------------------------------------------

#[test]
fn smart_routes_names_and_guards() {
    assert_eq!(route_name("/smart/preview"), "smart_preview");
    assert_eq!(route_name("/smart/rows"), "smart_rows");
    assert_eq!(route_name("/smart/apply"), "smart_apply");
    assert_eq!(route_name("/rules"), "rules");
    assert_eq!(route_name("/smart/preview/x"), "unknown");
    assert_eq!(handle(&no_db(), "POST", "/smart/preview/", "").0, 404);
}

#[test]
fn smart_preview_apply_undo_redo_through_the_api() {
    let (dir, _path, s) = loaded_state();
    let paint = cat_id(&s, "Paint");

    // preview: the selected row is a seed, the response is read-only
    let (code, body) = send(&s, "POST", "/smart/preview", &format!(r#"{{"ids":[2],"category_id":{paint}}}"#));
    assert_eq!(code, 200, "{body}");
    assert!(body["category"]["path"].as_str().unwrap().contains("Paint"));
    assert_eq!(body["threshold"], 0.75);
    assert_eq!(body["clusters"][0]["tier"], "seed");
    assert_eq!(body["clusters"][0]["txn_ids"], serde_json::json!([2]));
    assert_eq!(body["summary"]["selected"], 1);
    assert_eq!(body["summary"]["uncategorized_total"], 3);

    // errors: unknown category 404, no existing ids 400, bad threshold 400, bad body 400
    assert_eq!(send(&s, "POST", "/smart/preview", r#"{"ids":[2],"category_id":999999}"#).0, 404);
    let (c, b) = send(&s, "POST", "/smart/preview", &format!(r#"{{"ids":[999],"category_id":{paint}}}"#));
    assert_eq!((c, b["error"].as_str()), (400, Some("invalid")));
    assert_eq!(send(&s, "POST", "/smart/preview", &format!(r#"{{"ids":[2],"category_id":{paint},"threshold":1.5}}"#)).0, 400);
    assert_eq!(send(&s, "POST", "/smart/preview", "[1]").0, 400);
    assert_eq!(send(&s, "POST", "/smart/preview", &format!(r#"{{"ids":"2","category_id":{paint}}}"#)).0, 400);

    // rows: request order, unknown ids omitted, payee NULL shows as ""
    let (code, body) = send(&s, "POST", "/smart/rows", r#"{"ids":[3,999,1]}"#);
    assert_eq!(code, 200);
    let got: Vec<i64> = body["rows"].as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert_eq!(got, vec![3, 1]);
    assert_eq!(body["rows"][0]["payee"], "");

    // apply with a rule: one batch, cache reloaded before the response
    let (code, body) = send(
        &s,
        "POST",
        "/smart/apply",
        &format!(r#"{{"category_id":{paint},"txn_ids":[2],"overwrite":false,"make_rule":{{"name":"  B shop  "}}}}"#),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!((body["changed"].as_i64(), body["skipped"].as_i64(), body["skipped_conflicts"].as_i64()), (Some(1), Some(0), Some(0)));
    assert!(body["batch_id"].is_i64());
    assert!(body["rule_id"].is_i64());
    assert!(body["label"].as_str().unwrap().starts_with("Categorize 1 transaction as "));
    assert!(body["label"].as_str().unwrap().ends_with("(+rule)"));
    let (_, rows) = send(&s, "POST", "/smart/rows", r#"{"ids":[2]}"#);
    assert!(rows["rows"][0]["category_path"].as_str().unwrap().contains("Paint"), "cache reloaded");

    let (code, rules) = get(&s, "/rules");
    assert_eq!(code, 200);
    assert_eq!(rules["rules"].as_array().unwrap().len(), 1);
    assert_eq!(rules["rules"][0]["name"], "B shop");
    assert_eq!(rules["rules"][0]["match"]["payee_norm"], "b");
    assert_eq!(rules["rules"][0]["hits"], 0);

    // bad apply bodies
    assert_eq!(send(&s, "POST", "/smart/apply", &format!(r#"{{"category_id":{paint},"txn_ids":[]}}"#)).0, 400);
    assert_eq!(send(&s, "POST", "/smart/apply", &format!(r#"{{"category_id":{paint},"txn_ids":[2],"make_rule":5}}"#)).0, 400);
    assert_eq!(send(&s, "POST", "/smart/apply", &format!(r#"{{"category_id":{paint},"txn_ids":[2],"overwrite":"yes"}}"#)).0, 400);

    // one undo reverts the category and the rule together; redo brings them back
    assert_eq!(send(&s, "POST", "/undo", "").0, 200);
    assert_eq!(get(&s, "/rules").1["rules"], serde_json::json!([]));
    let (_, rows) = send(&s, "POST", "/smart/rows", r#"{"ids":[2]}"#);
    assert!(rows["rows"][0]["category_path"].is_null());
    assert_eq!(send(&s, "POST", "/redo", "").0, 200);
    assert_eq!(get(&s, "/rules").1["rules"].as_array().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
