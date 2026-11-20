# Multi-Level Category System

## Overview

The finances-tracker database now supports **three-level hierarchical categories** for more granular transaction classification:

- **Main:** Top-level category (e.g., "Home", "Utilities", "Taxes")
- **Sub:** Second-level subdivision (e.g., "Home/Improvement", "Utilities/Internet")
- **Subsub:** Third-level subdivision (e.g., for future use; currently unused in your list)

## Schema

### New Tables

```sql
CREATE TABLE category_main (
    id INT PRIMARY KEY,
    name TEXT UNIQUE,
    type TEXT DEFAULT 'Expense',  -- 'Expense' | 'Income' | 'Transfer'
    display_order INT,
    is_active INT DEFAULT 1,
    created_at DATETIME
);

CREATE TABLE category_sub (
    id INT PRIMARY KEY,
    main_id INT REFERENCES category_main(id),
    name TEXT,
    display_order INT,
    is_active INT DEFAULT 1,
    created_at DATETIME,
    UNIQUE(main_id, name)
);

CREATE TABLE category_subsub (
    id INT PRIMARY KEY,
    sub_id INT REFERENCES category_sub(id),
    name TEXT,
    display_order INT,
    is_active INT DEFAULT 1,
    created_at DATETIME,
    UNIQUE(sub_id, name)
);
```

### Updated Tables

`transactions` and `transaction_splits` now have three new columns:
- `category_main_id` — References `category_main(id)`
- `category_sub_id` — References `category_sub(id)`
- `category_subsub_id` — References `category_subsub(id)`

The original `category_id` column is kept for backward compatibility (references LM categories).

## Data Loaded

19 main categories with 16 sub-categories:
- **Education** → Services
- **Entertainment** → Books, Streaming media
- **Home** → Improvement, Maintenance, Mortgage, Security
- **Personal** → Hobbies, Maintenance, Vacations
- **Utilities** → Gas & Electric, Internet, Phone, Technology
- **Healthcare** → Doctor
- **Gifts** → Birthday or Christmas presents
- Plus 11 single-level categories (no subs): Geckos and snakes, Loans, Car Insurance, Gym, etc.

## Managing Categories

### View All Categories
```bash
python3 query_categories.py list
```

### View Main Categories Only
```bash
python3 query_categories.py list main
```

### Add a New Main Category
```bash
python3 query_categories.py add-main "Category Name"
```

### Add a New Sub Category
```bash
python3 query_categories.py add-sub <main_id> "Sub Name"
```

### Add a New Subsub Category
```bash
python3 query_categories.py add-subsub <sub_id> "Subsub Name"
```

### Search Categories
```bash
python3 query_categories.py search "keyword"
```

## Querying Transactions by Category

### All transactions in a main category
```sql
SELECT * FROM transactions
WHERE category_main_id = 5  -- e.g., Home
ORDER BY date DESC;
```

### All transactions in a sub category
```sql
SELECT * FROM transactions
WHERE category_sub_id = 7  -- e.g., Home/Mortgage
ORDER BY date DESC;
```

### Transactions with full category hierarchy
```sql
SELECT 
    t.date, t.payee, t.amount,
    m.name as main_category,
    s.name as sub_category,
    ss.name as subsub_category
FROM transactions t
LEFT JOIN category_main m ON t.category_main_id = m.id
LEFT JOIN category_sub s ON t.category_sub_id = s.id
LEFT JOIN category_subsub ss ON t.category_subsub_id = ss.id
ORDER BY t.date DESC
LIMIT 20;
```

## Future: Transaction Assignment

To assign transactions to the new categories, you can:

1. **Bulk assign** based on keywords or existing LM categories
2. **Split transactions** into multiple category levels via `transaction_splits` table
3. **Create custom rules** to auto-assign sub-categories based on payee/merchant

These workflows are planned as separate future tasks.

## Files

- `migration_multilevel_categories.sql` — Schema migration (already applied)
- `categories.json` — Category hierarchy (flat JSON, editable)
- `load_categories.py` — Bulk loader (already executed)
- `query_categories.py` — CLI tool for category management
