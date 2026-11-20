#!/usr/bin/env python3
"""
Finances Tracker — Lunch Money sync script
Runs weekly (Sunday midnight via Hermes cron).
Pulls all unseen transactions + accounts, upserts into encrypted SQLCipher DB.
Intelligent catch-up: always syncs from the day after the last successful sync,
so a missed week is automatically recovered on the next run.
"""

import json
import logging
import os
import sys
import time
from datetime import date, datetime, timedelta
from pathlib import Path

import requests
from dotenv import load_dotenv

# ---------------------------------------------------------------------------
# Bootstrap
# ---------------------------------------------------------------------------

SCRIPT_DIR = Path(__file__).parent.resolve()
load_dotenv(SCRIPT_DIR / ".env")

API_TOKEN   = os.environ["LUNCHMONEY_API_TOKEN"]
DB_PASS     = os.environ["DB_PASSPHRASE"]
DB_PATH     = SCRIPT_DIR / "finances.db"
LOG_PATH    = SCRIPT_DIR / "sync.log"
SCHEMA_PATH = SCRIPT_DIR / "schema.sql"

# Lunch Money API
LM_V2 = "https://api.lunchmoney.dev/v2"
LM_V1 = "https://api.lunchmoney.dev/v1"   # accounts / plaid_accounts still live on v1
HEADERS = {"Authorization": f"Bearer {API_TOKEN}"}

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[
        logging.FileHandler(LOG_PATH),
        logging.StreamHandler(sys.stdout),
    ],
)
log = logging.getLogger("sync")

# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------

def open_db():
    """Open (or create) the encrypted SQLCipher database."""
    try:
        from sqlcipher3 import dbapi2 as sqlite
    except ImportError:
        log.error("sqlcipher3 not installed — run: pip install sqlcipher3-binary")
        sys.exit(1)

    conn = sqlite.connect(str(DB_PATH))
    conn.execute(f"PRAGMA key='{DB_PASS}'")
    conn.execute("PRAGMA foreign_keys = ON")
    conn.execute("PRAGMA journal_mode = WAL")
    return conn


def init_schema(conn):
    """Apply schema.sql (idempotent — all CREATE IF NOT EXISTS)."""
    import re
    sql = SCHEMA_PATH.read_text()
    # Strip full-line and inline -- comments before splitting
    sql = re.sub(r"--[^\n]*", "", sql)
    for stmt in sql.split(";"):
        stmt = stmt.strip()
        if not stmt:
            continue
        try:
            conn.execute(stmt)
        except Exception as e:
            msg = str(e)
            if "duplicate column" in msg.lower():
                continue
            raise
    conn.commit()
    log.info("Schema initialised.")


# ---------------------------------------------------------------------------
# Lunch Money API helpers
# ---------------------------------------------------------------------------

def lm_get(url, params=None, retries=3):
    """GET with retry/backoff."""
    for attempt in range(retries):
        try:
            r = requests.get(url, headers=HEADERS, params=params, timeout=30)
            if r.status_code == 429:
                retry_after = int(r.headers.get("Retry-After", 60))
                log.warning(f"Rate limited — sleeping {retry_after}s")
                time.sleep(retry_after)
                continue
            r.raise_for_status()
            return r.json()
        except requests.RequestException as e:
            if attempt == retries - 1:
                raise
            wait = 2 ** attempt
            log.warning(f"Request failed ({e}), retrying in {wait}s…")
            time.sleep(wait)


# ---------------------------------------------------------------------------
# Sync: categories
# ---------------------------------------------------------------------------

def sync_categories(conn):
    """Fetch all categories from LM v2 and upsert."""
    log.info("Fetching categories…")
    data = lm_get(f"{LM_V2}/categories", {"format": "flattened"})
    cats = data.get("categories", [])
    log.info(f"  {len(cats)} categories received.")

    # Insert group (parent) categories first to satisfy FK constraint
    cats.sort(key=lambda c: (1 if c.get("group_id") else 0))

    upserted = 0
    for c in cats:
        conn.execute(
            """
            INSERT INTO categories (
                id, name, description, is_group, group_id, group_name,
                is_income, exclude_from_budget, exclude_from_totals,
                archived, archived_on, order_index,
                created_at_lm, updated_at_lm, synced_at
            ) VALUES (
                :id, :name, :description, :is_group, :group_id, :group_name,
                :is_income, :exclude_from_budget, :exclude_from_totals,
                :archived, :archived_on, :order_index,
                :created_at, :updated_at, datetime('now')
            )
            ON CONFLICT(id) DO UPDATE SET
                name                 = excluded.name,
                description          = excluded.description,
                is_group             = excluded.is_group,
                group_id             = excluded.group_id,
                group_name           = excluded.group_name,
                is_income            = excluded.is_income,
                exclude_from_budget  = excluded.exclude_from_budget,
                exclude_from_totals  = excluded.exclude_from_totals,
                archived             = excluded.archived,
                archived_on          = excluded.archived_on,
                order_index          = excluded.order_index,
                updated_at_lm        = excluded.updated_at_lm,
                synced_at            = datetime('now')
            """,
            {
                "id":                   c["id"],
                "name":                 c.get("name"),
                "description":          c.get("description"),
                "is_group":             1 if c.get("is_group") else 0,
                "group_id":             c.get("group_id"),
                "group_name":           c.get("group_name"),
                "is_income":            1 if c.get("is_income") else 0,
                "exclude_from_budget":  1 if c.get("exclude_from_budget") else 0,
                "exclude_from_totals":  1 if c.get("exclude_from_totals") else 0,
                "archived":             1 if c.get("archived") else 0,
                "archived_on":          c.get("archived_on"),
                "order_index":          c.get("order"),
                "created_at":           c.get("created_at"),
                "updated_at":           c.get("updated_at"),
            },
        )
        upserted += 1

    conn.commit()
    log.info(f"  Categories upserted: {upserted}")
    return len(cats), upserted


# ---------------------------------------------------------------------------
# Sync: accounts + balance snapshots
# ---------------------------------------------------------------------------

def sync_accounts(conn, today: str):
    """Fetch plaid accounts (v1) and upsert; snapshot today's balance."""
    log.info("Fetching accounts…")
    data = lm_get(f"{LM_V1}/plaid_accounts")
    accts = data.get("plaid_accounts", [])

    # Also fetch manual assets (v1)
    asset_data = lm_get(f"{LM_V1}/assets")
    assets = asset_data.get("assets", [])

    log.info(f"  {len(accts)} plaid accounts, {len(assets)} manual assets.")

    upserted = 0
    balances_recorded = 0

    for a in accts:
        conn.execute(
            """
            INSERT INTO accounts (
                id, account_type, name, display_name, type, subtype,
                institution_name, mask, status, currency, credit_limit,
                is_closed, import_start_date, date_linked,
                updated_at_lm, synced_at
            ) VALUES (
                :id, 'plaid', :name, :display_name, :type, :subtype,
                :institution_name, :mask, :status, :currency, :credit_limit,
                0, :import_start_date, :date_linked,
                :updated_at, datetime('now')
            )
            ON CONFLICT(id) DO UPDATE SET
                name              = excluded.name,
                display_name      = excluded.display_name,
                type              = excluded.type,
                subtype           = excluded.subtype,
                institution_name  = excluded.institution_name,
                mask              = excluded.mask,
                status            = excluded.status,
                currency          = excluded.currency,
                credit_limit      = excluded.credit_limit,
                import_start_date = excluded.import_start_date,
                updated_at_lm     = excluded.updated_at_lm,
                synced_at         = datetime('now')
            """,
            {
                "id":               a["id"],
                "name":             a.get("name"),
                "display_name":     a.get("display_name"),
                "type":             a.get("type"),
                "subtype":          a.get("subtype"),
                "institution_name": a.get("institution_name"),
                "mask":             a.get("mask"),
                "status":           a.get("status"),
                "currency":         a.get("currency"),
                "credit_limit":     a.get("limit"),
                "import_start_date":a.get("import_start_date"),
                "date_linked":      a.get("date_linked"),
                "updated_at":       a.get("balance_last_update"),
            },
        )
        upserted += 1

        # Balance snapshot
        bal = a.get("balance")
        if bal is not None:
            conn.execute(
                """
                INSERT INTO account_balances (account_id, balance_date, balance, currency, to_base, source)
                VALUES (:acct, :dt, :bal, :cur, :base, 'sync')
                ON CONFLICT(account_id, balance_date) DO UPDATE SET
                    balance  = excluded.balance,
                    to_base  = excluded.to_base
                """,
                {
                    "acct": a["id"],
                    "dt":   today,
                    "bal":  float(bal),
                    "cur":  a.get("currency"),
                    "base": a.get("to_base"),
                },
            )
            balances_recorded += 1

    # Manual assets
    for a in assets:
        conn.execute(
            """
            INSERT INTO accounts (
                id, account_type, name, display_name, type, subtype,
                institution_name, currency, is_closed, synced_at
            ) VALUES (
                :id, 'asset', :name, :display_name, :type, :subtype,
                :institution_name, :currency, :is_closed, datetime('now')
            )
            ON CONFLICT(id) DO UPDATE SET
                name             = excluded.name,
                display_name     = excluded.display_name,
                type             = excluded.type,
                subtype          = excluded.subtype,
                institution_name = excluded.institution_name,
                currency         = excluded.currency,
                is_closed        = excluded.is_closed,
                synced_at        = datetime('now')
            """,
            {
                "id":               a["id"],
                "name":             a.get("name"),
                "display_name":     a.get("display_name"),
                "type":             a.get("type_name"),
                "subtype":          a.get("subtype_name"),
                "institution_name": a.get("institution_name"),
                "currency":         a.get("currency"),
                "is_closed":        1 if a.get("closed_on") else 0,
            },
        )
        upserted += 1

        bal = a.get("balance")
        if bal is not None:
            conn.execute(
                """
                INSERT INTO account_balances (account_id, balance_date, balance, currency, to_base, source)
                VALUES (:acct, :dt, :bal, :cur, :base, 'sync')
                ON CONFLICT(account_id, balance_date) DO UPDATE SET
                    balance  = excluded.balance,
                    to_base  = excluded.to_base
                """,
                {
                    "acct": a["id"],
                    "dt":   today,
                    "bal":  float(bal),
                    "cur":  a.get("currency"),
                    "base": a.get("to_base"),
                },
            )
            balances_recorded += 1

    conn.commit()
    log.info(f"  Accounts upserted: {upserted}, balance snapshots: {balances_recorded}")
    return len(accts) + len(assets), upserted, balances_recorded


# ---------------------------------------------------------------------------
# Sync: transactions
# ---------------------------------------------------------------------------

TRANSACTIONS_PER_PAGE = 500


def get_last_sync_date(conn) -> str | None:
    """Return the fetch_to_date of the last *successful* sync, or None."""
    row = conn.execute(
        """
        SELECT fetch_to_date FROM sync_log
        WHERE status = 'success' AND fetch_to_date IS NOT NULL
        ORDER BY id DESC LIMIT 1
        """
    ).fetchone()
    return row[0] if row else None


def sync_transactions(conn, start_date: str, end_date: str):
    """
    Fetch all transactions between start_date and end_date (inclusive),
    paginating through results.  Upserts on conflict so re-runs are safe.
    """
    log.info(f"Fetching transactions {start_date} → {end_date}…")

    fetched_total = 0
    inserted = 0
    updated = 0
    offset = 0

    while True:
        params = {
            "start_date": start_date,
            "end_date":   end_date,
            "limit":      TRANSACTIONS_PER_PAGE,
            "offset":     offset,
        }
        data = lm_get(f"{LM_V2}/transactions", params)
        txns = data.get("transactions", [])
        has_more = data.get("has_more", False)

        for t in txns:
            # Check whether we've seen this id before (to distinguish insert vs update)
            existing = conn.execute(
                "SELECT id FROM transactions WHERE id = ?", (t["id"],)
            ).fetchone()

            conn.execute(
                """
                INSERT INTO transactions (
                    id, date, payee, original_name, amount, currency, to_base,
                    notes, status, is_pending,
                    category_id, plaid_account_id, manual_account_id, source,
                    tag_ids,
                    is_group_parent, group_parent_id,
                    is_split_parent, split_parent_id,
                    recurring_id, external_id,
                    created_at_lm, updated_at_lm, synced_at
                ) VALUES (
                    :id, :date, :payee, :original_name, :amount, :currency, :to_base,
                    :notes, :status, :is_pending,
                    :category_id, :plaid_account_id, :manual_account_id, :source,
                    :tag_ids,
                    :is_group_parent, :group_parent_id,
                    :is_split_parent, :split_parent_id,
                    :recurring_id, :external_id,
                    :created_at, :updated_at, datetime('now')
                )
                ON CONFLICT(id) DO UPDATE SET
                    date             = excluded.date,
                    payee            = excluded.payee,
                    original_name    = excluded.original_name,
                    amount           = excluded.amount,
                    currency         = excluded.currency,
                    to_base          = excluded.to_base,
                    notes            = excluded.notes,
                    status           = excluded.status,
                    is_pending       = excluded.is_pending,
                    category_id      = excluded.category_id,
                    plaid_account_id = excluded.plaid_account_id,
                    manual_account_id= excluded.manual_account_id,
                    source           = excluded.source,
                    tag_ids          = excluded.tag_ids,
                    is_group_parent  = excluded.is_group_parent,
                    group_parent_id  = excluded.group_parent_id,
                    is_split_parent  = excluded.is_split_parent,
                    split_parent_id  = excluded.split_parent_id,
                    recurring_id     = excluded.recurring_id,
                    external_id      = excluded.external_id,
                    updated_at_lm    = excluded.updated_at_lm,
                    synced_at        = datetime('now')
                """,
                {
                    "id":               t["id"],
                    "date":             t.get("date"),
                    "payee":            t.get("payee"),
                    "original_name":    t.get("original_name"),
                    "amount":           float(t.get("amount", 0)),
                    "currency":         t.get("currency"),
                    "to_base":          t.get("to_base"),
                    "notes":            t.get("notes"),
                    "status":           t.get("status"),
                    "is_pending":       1 if t.get("is_pending") else 0,
                    "category_id":      t.get("category_id"),
                    "plaid_account_id": t.get("plaid_account_id"),
                    "manual_account_id":t.get("manual_account_id"),
                    "source":           t.get("source"),
                    "tag_ids":          json.dumps(t.get("tag_ids", [])),
                    "is_group_parent":  1 if t.get("is_group_parent") else 0,
                    "group_parent_id":  t.get("group_parent_id"),
                    "is_split_parent":  1 if t.get("is_split_parent") else 0,
                    "split_parent_id":  t.get("split_parent_id"),
                    "recurring_id":     t.get("recurring_id"),
                    "external_id":      t.get("external_id"),
                    "created_at":       t.get("created_at"),
                    "updated_at":       t.get("updated_at"),
                },
            )

            if existing:
                updated += 1
            else:
                inserted += 1

        fetched_total += len(txns)
        log.info(f"  Page offset={offset}: {len(txns)} transactions")

        if not has_more or not txns:
            break
        offset += TRANSACTIONS_PER_PAGE

    conn.commit()
    log.info(f"  Transactions: {fetched_total} fetched, {inserted} new, {updated} updated")
    return fetched_total, inserted, updated


# ---------------------------------------------------------------------------
# Main sync orchestration
# ---------------------------------------------------------------------------

def run_sync():
    today = date.today().isoformat()
    log.info("=" * 60)
    log.info(f"Sync started: {datetime.now().isoformat()}")

    conn = open_db()
    init_schema(conn)

    # Determine date range — catch-up aware
    last_sync = get_last_sync_date(conn)
    if last_sync:
        # Start day after last successful sync
        start_date = (date.fromisoformat(last_sync) + timedelta(days=1)).isoformat()
        log.info(f"Last successful sync was to {last_sync}; fetching from {start_date}")
    else:
        # First-ever run: pull everything LM has (LM launched ~2019; use a wide window)
        start_date = "2019-01-01"
        log.info(f"No prior sync found — full historical backfill from {start_date}")

    # Open sync_log row
    cur = conn.execute(
        """
        INSERT INTO sync_log (started_at, fetch_from_date, fetch_to_date, status)
        VALUES (datetime('now'), ?, ?, 'running')
        """,
        (start_date, today),
    )
    sync_id = cur.lastrowid
    conn.commit()

    try:
        # 1. Categories
        cats_fetched, cats_upserted = sync_categories(conn)

        # 2. Accounts + balance snapshots
        accts_fetched, accts_upserted, balances = sync_accounts(conn, today)

        # 3. Transactions
        tx_fetched, tx_inserted, tx_updated = sync_transactions(conn, start_date, today)

        # 4. Mark success
        conn.execute(
            """
            UPDATE sync_log SET
                finished_at           = datetime('now'),
                transactions_fetched  = ?,
                transactions_inserted = ?,
                transactions_updated  = ?,
                accounts_fetched      = ?,
                accounts_upserted     = ?,
                balances_recorded     = ?,
                categories_fetched    = ?,
                categories_upserted   = ?,
                status                = 'success'
            WHERE id = ?
            """,
            (tx_fetched, tx_inserted, tx_updated,
             accts_fetched, accts_upserted, balances,
             cats_fetched, cats_upserted,
             sync_id),
        )
        conn.commit()
        log.info(f"Sync complete. New transactions: {tx_inserted}, updated: {tx_updated}")

    except Exception as e:
        conn.execute(
            "UPDATE sync_log SET finished_at=datetime('now'), status='error', error_message=? WHERE id=?",
            (str(e), sync_id),
        )
        conn.commit()
        log.error(f"Sync FAILED: {e}")
        raise
    finally:
        conn.close()


if __name__ == "__main__":
    run_sync()
