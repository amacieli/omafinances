#!/usr/bin/env python3
"""
Query and manage multi-level categories in finances-tracker.

Usage:
    python3 query_categories.py list           # Show all categories
    python3 query_categories.py list main      # Show main categories only
    python3 query_categories.py add-main "NAME"
    python3 query_categories.py add-sub <main_id> "SUB_NAME"
    python3 query_categories.py add-subsub <sub_id> "SUBSUB_NAME"
    python3 query_categories.py search "keyword"
"""

import sys
import sqlite3
from pathlib import Path
from dotenv import load_dotenv
import os

load_dotenv(Path(__file__).parent / '.env')
DB_PASSPHRASE = os.environ.get('DB_PASSPHRASE')

if not DB_PASSPHRASE:
    print("ERROR: DB_PASSPHRASE not found in .env", file=sys.stderr)
    sys.exit(1)

def connect_db():
    try:
        from sqlcipher3 import dbapi2 as sqlite
    except ImportError:
        print("ERROR: sqlcipher3 not installed", file=sys.stderr)
        sys.exit(1)
    
    conn = sqlite.connect(str(Path(__file__).parent / 'finances.db'))
    conn.execute(f"PRAGMA key='{DB_PASSPHRASE}'")
    conn.execute("PRAGMA foreign_keys = ON")
    return conn

def list_categories(conn, level=None):
    """Display category hierarchy."""
    cur = conn.cursor()
    
    if level == 'main':
        cur.execute("""
            SELECT id, name, type, is_active, created_at
            FROM category_main
            ORDER BY display_order, name
        """)
        print(f"{'ID':<4} {'Name':<40} {'Type':<10} {'Active':<6} {'Created':<10}")
        print("-" * 80)
        for row_id, name, type_, is_active, created in cur.fetchall():
            active = "Yes" if is_active else "No"
            created_date = created.split('T')[0] if created else "---"
            print(f"{row_id:<4} {name:<40} {type_:<10} {active:<6} {created_date:<10}")
    else:
        # Full hierarchy
        cur.execute("""
            SELECT 
                m.id as m_id,
                m.name as main,
                s.id as s_id,
                s.name as sub,
                ss.id as ss_id,
                ss.name as subsub
            FROM category_main m
            LEFT JOIN category_sub s ON s.main_id = m.id
            LEFT JOIN category_subsub ss ON ss.sub_id = s.id
            ORDER BY m.display_order, m.name, s.display_order, s.name, ss.display_order, ss.name
        """)
        
        prev_main = None
        for m_id, main, s_id, sub, ss_id, subsub in cur.fetchall():
            if main != prev_main:
                print(f"\n[{m_id:2d}] {main}")
                prev_main = main
            if sub:
                if subsub:
                    print(f"    ├─ [{s_id:2d}] {sub}")
                    print(f"       └─ [{ss_id:2d}] {subsub}")
                else:
                    print(f"    └─ [{s_id:2d}] {sub}")

def add_main(conn, name):
    """Add a new main category."""
    cur = conn.cursor()
    try:
        cur.execute("""
            INSERT INTO category_main (name, type)
            VALUES (?, 'Expense')
        """, (name,))
        conn.commit()
        new_id = cur.lastrowid
        print(f"✓ Added main category '{name}' (id={new_id})")
        return new_id
    except sqlite3.IntegrityError:
        print(f"✗ Main category '{name}' already exists", file=sys.stderr)
        return None

def add_sub(conn, main_id, name):
    """Add a new sub category."""
    cur = conn.cursor()
    try:
        cur.execute("""
            INSERT INTO category_sub (main_id, name)
            VALUES (?, ?)
        """, (main_id, name))
        conn.commit()
        new_id = cur.lastrowid
        
        # Show parent
        cur.execute("SELECT name FROM category_main WHERE id = ?", (main_id,))
        parent = cur.fetchone()
        if parent:
            print(f"✓ Added sub category '{name}' under '{parent[0]}' (id={new_id})")
        return new_id
    except sqlite3.IntegrityError:
        print(f"✗ Sub category '{name}' already exists under main id {main_id}", file=sys.stderr)
        return None
    except sqlite3.OperationalError as e:
        print(f"✗ Error: {e}", file=sys.stderr)
        return None

def add_subsub(conn, sub_id, name):
    """Add a new subsub category."""
    cur = conn.cursor()
    try:
        cur.execute("""
            INSERT INTO category_subsub (sub_id, name)
            VALUES (?, ?)
        """, (sub_id, name))
        conn.commit()
        new_id = cur.lastrowid
        
        # Show parent chain
        cur.execute("""
            SELECT m.name, s.name
            FROM category_sub s
            JOIN category_main m ON s.main_id = m.id
            WHERE s.id = ?
        """, (sub_id,))
        parent = cur.fetchone()
        if parent:
            print(f"✓ Added subsub category '{name}' under '{parent[0]} > {parent[1]}' (id={new_id})")
        return new_id
    except sqlite3.IntegrityError:
        print(f"✗ Subsub category '{name}' already exists under sub id {sub_id}", file=sys.stderr)
        return None

def search_categories(conn, keyword):
    """Search categories by keyword."""
    cur = conn.cursor()
    keyword = f"%{keyword}%"
    
    cur.execute("""
        SELECT 'main' as level, id, name FROM category_main WHERE name LIKE ?
        UNION
        SELECT 'sub' as level, id, name FROM category_sub WHERE name LIKE ?
        UNION
        SELECT 'subsub' as level, id, name FROM category_subsub WHERE name LIKE ?
        ORDER BY level, name
    """, (keyword, keyword, keyword))
    
    results = cur.fetchall()
    if not results:
        print(f"No categories match '{keyword}'")
        return
    
    print(f"Search results for '{keyword}':")
    print("-" * 50)
    for level, cat_id, name in results:
        print(f"[{level:6s}] id={cat_id:2d} {name}")

if __name__ == '__main__':
    conn = connect_db()
    
    if len(sys.argv) < 2:
        print(__doc__)
        list_categories(conn)
    elif sys.argv[1] == 'list':
        level = sys.argv[2] if len(sys.argv) > 2 else None
        list_categories(conn, level=level)
    elif sys.argv[1] == 'add-main':
        if len(sys.argv) < 3:
            print("Usage: add-main 'NAME'")
            sys.exit(1)
        add_main(conn, sys.argv[2])
    elif sys.argv[1] == 'add-sub':
        if len(sys.argv) < 4:
            print("Usage: add-sub <main_id> 'NAME'")
            sys.exit(1)
        add_sub(conn, int(sys.argv[2]), sys.argv[3])
    elif sys.argv[1] == 'add-subsub':
        if len(sys.argv) < 4:
            print("Usage: add-subsub <sub_id> 'NAME'")
            sys.exit(1)
        add_subsub(conn, int(sys.argv[2]), sys.argv[3])
    elif sys.argv[1] == 'search':
        if len(sys.argv) < 3:
            print("Usage: search 'keyword'")
            sys.exit(1)
        search_categories(conn, sys.argv[2])
    else:
        print(f"Unknown command: {sys.argv[1]}")
        print(__doc__)
    
    conn.close()
