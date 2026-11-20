-- ============================================================
-- Finances Tracker Schema
-- SQLCipher AES-256 encrypted database
-- ============================================================

PRAGMA foreign_keys = ON;

-- ============================================================
-- accounts
-- Plaid-linked and manual accounts from Lunch Money
-- ============================================================
CREATE TABLE IF NOT EXISTS accounts (
    id                  INTEGER PRIMARY KEY,   -- Lunch Money plaid_account or asset id
    account_type        TEXT    NOT NULL DEFAULT 'plaid',  -- 'plaid' | 'asset'
    name                TEXT    NOT NULL,
    display_name        TEXT,
    type                TEXT,                  -- credit, depository, investment, loan, etc.
    subtype             TEXT,                  -- credit card, checking, savings, etc.
    institution_name    TEXT,
    mask                TEXT,                  -- last 4 digits
    status              TEXT,                  -- active, inactive, relink, syncing, error, not found, deleted
    currency            TEXT,
    credit_limit        REAL,
    is_closed           INTEGER NOT NULL DEFAULT 0,
    import_start_date   TEXT,
    date_linked         TEXT,
    created_at_lm       TEXT,
    updated_at_lm       TEXT,
    synced_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ============================================================
-- categories
-- Lunch Money categories; supports hierarchy (group -> child)
-- Split feature: local-only categories can be created (id can be NULL for those)
-- ============================================================
CREATE TABLE IF NOT EXISTS categories (
    id                      INTEGER PRIMARY KEY,   -- LM category id (NULL = local-only future use)
    name                    TEXT    NOT NULL,
    description             TEXT,
    is_group                INTEGER NOT NULL DEFAULT 0,
    group_id                INTEGER REFERENCES categories(id),
    group_name              TEXT,
    is_income               INTEGER NOT NULL DEFAULT 0,
    exclude_from_budget     INTEGER NOT NULL DEFAULT 0,
    exclude_from_totals     INTEGER NOT NULL DEFAULT 0,
    archived                INTEGER NOT NULL DEFAULT 0,
    archived_on             TEXT,
    order_index             INTEGER,
    created_at_lm           TEXT,
    updated_at_lm           TEXT,
    synced_at               TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ============================================================
-- transactions
-- Raw Lunch Money transactions (v2 API field names)
-- ============================================================
CREATE TABLE IF NOT EXISTS transactions (
    id                  INTEGER PRIMARY KEY,   -- Lunch Money transaction id
    date                TEXT    NOT NULL,      -- ISO-8601 date YYYY-MM-DD
    payee               TEXT,
    original_name       TEXT,                  -- institution-provided name
    amount              REAL    NOT NULL,      -- positive = debit, negative = credit (LM v2)
    currency            TEXT,
    to_base             REAL,                  -- amount converted to primary currency
    notes               TEXT,
    status              TEXT,                  -- unreviewed, cleared, pending, recurring, recurring_suggested
    is_pending          INTEGER NOT NULL DEFAULT 0,
    category_id         INTEGER REFERENCES categories(id),
    plaid_account_id    INTEGER REFERENCES accounts(id),
    manual_account_id   INTEGER REFERENCES accounts(id),
    source              TEXT,                  -- plaid, manual, api, etc.
    tag_ids             TEXT,                  -- JSON array of tag ids
    -- Group transaction support
    is_group_parent     INTEGER NOT NULL DEFAULT 0,
    group_parent_id     INTEGER REFERENCES transactions(id),
    -- Split support (future): is_split_parent marks rows that have been split locally
    is_split_parent     INTEGER NOT NULL DEFAULT 0,
    split_parent_id     INTEGER REFERENCES transactions(id),
    -- Recurring
    recurring_id        INTEGER,
    -- External
    external_id         TEXT,
    -- Audit
    created_at_lm       TEXT,
    updated_at_lm       TEXT,
    synced_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_transactions_date             ON transactions(date);
CREATE INDEX IF NOT EXISTS idx_transactions_plaid_account    ON transactions(plaid_account_id);
CREATE INDEX IF NOT EXISTS idx_transactions_manual_account   ON transactions(manual_account_id);
CREATE INDEX IF NOT EXISTS idx_transactions_category         ON transactions(category_id);
CREATE INDEX IF NOT EXISTS idx_transactions_split_parent     ON transactions(split_parent_id);
CREATE INDEX IF NOT EXISTS idx_transactions_group_parent     ON transactions(group_parent_id);
CREATE INDEX IF NOT EXISTS idx_transactions_updated_at_lm    ON transactions(updated_at_lm);

-- ============================================================
-- transaction_splits
-- Future: manually split one transaction into sub-amounts + categories
-- Parent transaction is flagged is_split_parent=1; amounts should sum to parent.
-- ============================================================
CREATE TABLE IF NOT EXISTS transaction_splits (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_tx_id    INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    description     TEXT    NOT NULL,
    amount          REAL    NOT NULL,
    category_id     INTEGER REFERENCES categories(id),
    notes           TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_splits_parent ON transaction_splits(parent_tx_id);

-- ============================================================
-- account_balances
-- Periodic balance snapshots per account for trend tracking
-- One row per account per day; upsert on conflict keeps latest.
-- ============================================================
CREATE TABLE IF NOT EXISTS account_balances (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      INTEGER NOT NULL REFERENCES accounts(id),
    balance_date    TEXT    NOT NULL,       -- ISO-8601 date
    balance         REAL    NOT NULL,
    currency        TEXT,
    to_base         REAL,                   -- balance in primary currency
    source          TEXT NOT NULL DEFAULT 'sync',  -- 'sync' | 'manual'
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, balance_date)
);

CREATE INDEX IF NOT EXISTS idx_balances_account_date ON account_balances(account_id, balance_date);

-- ============================================================
-- sync_log
-- One row per sync run; status updated on completion or failure
-- ============================================================
CREATE TABLE IF NOT EXISTS sync_log (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at              TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at             TEXT,
    fetch_from_date         TEXT,           -- earliest date queried
    fetch_to_date           TEXT,           -- latest date queried (usually today)
    transactions_fetched    INTEGER DEFAULT 0,
    transactions_inserted   INTEGER DEFAULT 0,
    transactions_updated    INTEGER DEFAULT 0,
    accounts_fetched        INTEGER DEFAULT 0,
    accounts_upserted       INTEGER DEFAULT 0,
    balances_recorded       INTEGER DEFAULT 0,
    categories_fetched      INTEGER DEFAULT 0,
    categories_upserted     INTEGER DEFAULT 0,
    status                  TEXT NOT NULL DEFAULT 'running',  -- running | success | error
    error_message           TEXT
);
