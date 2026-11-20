-- ============================================================
-- Migration: Multi-level Category System
-- Adds support for main / sub / sub-sub categories
-- ============================================================

PRAGMA foreign_keys = ON;

-- ============================================================
-- category_main
-- Top-level categories (e.g., "Home", "Education", "Taxes")
-- ============================================================
CREATE TABLE IF NOT EXISTS category_main (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT    NOT NULL UNIQUE,
    description     TEXT,
    type            TEXT    NOT NULL DEFAULT 'Expense',  -- 'Expense' | 'Income' | 'Transfer'
    display_order   INTEGER DEFAULT 0,
    is_active       INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ============================================================
-- category_sub
-- Sub-level categories (e.g., "Home/Improvement", "Education/Services")
-- ============================================================
CREATE TABLE IF NOT EXISTS category_sub (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    main_id         INTEGER NOT NULL REFERENCES category_main(id) ON DELETE CASCADE,
    name            TEXT    NOT NULL,
    description     TEXT,
    display_order   INTEGER DEFAULT 0,
    is_active       INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(main_id, name)
);

-- ============================================================
-- category_subsub
-- Sub-sub-level categories (e.g., "Taxes/Federal/Form 1040")
-- ============================================================
CREATE TABLE IF NOT EXISTS category_subsub (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    sub_id          INTEGER NOT NULL REFERENCES category_sub(id) ON DELETE CASCADE,
    name            TEXT    NOT NULL,
    description     TEXT,
    display_order   INTEGER DEFAULT 0,
    is_active       INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(sub_id, name)
);

-- ============================================================
-- Add category level columns to transactions
-- Replaces the single category_id with explicit level references
-- Old category_id kept for backward compat (for now)
-- ============================================================
ALTER TABLE transactions ADD COLUMN category_main_id INTEGER REFERENCES category_main(id);
ALTER TABLE transactions ADD COLUMN category_sub_id INTEGER REFERENCES category_sub(id);
ALTER TABLE transactions ADD COLUMN category_subsub_id INTEGER REFERENCES category_subsub(id);

-- ============================================================
-- Add category level columns to transaction_splits
-- Same three-level structure
-- ============================================================
ALTER TABLE transaction_splits ADD COLUMN category_main_id INTEGER REFERENCES category_main(id);
ALTER TABLE transaction_splits ADD COLUMN category_sub_id INTEGER REFERENCES category_sub(id);
ALTER TABLE transaction_splits ADD COLUMN category_subsub_id INTEGER REFERENCES category_subsub(id);

-- ============================================================
-- Create indexes for fast lookups
-- ============================================================
CREATE INDEX IF NOT EXISTS idx_category_main_active ON category_main(is_active);
CREATE INDEX IF NOT EXISTS idx_category_sub_main ON category_sub(main_id);
CREATE INDEX IF NOT EXISTS idx_category_subsub_sub ON category_subsub(sub_id);

CREATE INDEX IF NOT EXISTS idx_transactions_cat_main ON transactions(category_main_id);
CREATE INDEX IF NOT EXISTS idx_transactions_cat_sub ON transactions(category_sub_id);
CREATE INDEX IF NOT EXISTS idx_transactions_cat_subsub ON transactions(category_subsub_id);

CREATE INDEX IF NOT EXISTS idx_splits_cat_main ON transaction_splits(category_main_id);
CREATE INDEX IF NOT EXISTS idx_splits_cat_sub ON transaction_splits(category_sub_id);
CREATE INDEX IF NOT EXISTS idx_splits_cat_subsub ON transaction_splits(category_subsub_id);
