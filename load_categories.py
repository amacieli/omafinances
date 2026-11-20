#!/usr/bin/env python3
"""
Load multi-level categories from a JSON structure into the finances-tracker database.

Usage:
    python3 load_categories.py

Reads: categories.json (JSON hierarchy exported from your category list)
Writes: Populates category_main, category_sub, category_subsub tables
"""

import json
import sqlite3
import sys
from pathlib import Path
from dotenv import load_dotenv
import os

# Load secrets
load_dotenv(Path(__file__).parent / '.env')
DB_PASSPHRASE = os.environ.get('DB_PASSPHRASE')
if not DB_PASSPHRASE:
    print("ERROR: DB_PASSPHRASE not found in .env", file=sys.stderr)
    sys.exit(1)

DB_PATH = Path(__file__).parent / 'finances.db'

def connect_db():
    """Open encrypted SQLCipher connection."""
    try:
        from sqlcipher3 import dbapi2 as sqlite
    except ImportError:
        print("ERROR: sqlcipher3 not installed. Run: pip install sqlcipher3", file=sys.stderr)
        sys.exit(1)
    
    conn = sqlite.connect(str(DB_PATH))
    conn.execute(f"PRAGMA key='{DB_PASSPHRASE}'")
    conn.execute("PRAGMA foreign_keys = ON")
    return conn

def load_categories_json():
    """Load category structure from stdin or categories.json file."""
    json_path = Path(__file__).parent / 'categories.json'
    
    if json_path.exists():
        with open(json_path, 'r') as f:
            return json.load(f)
    else:
        print(f"ERROR: {json_path} not found", file=sys.stderr)
        print("Create categories.json with structure:", file=sys.stderr)
        print(json.dumps({
            "main": {
                "Education": {
                    "subs": {
                        "Services": {"subsubs": {}}
                    }
                }
            }
        }, indent=2), file=sys.stderr)
        sys.exit(1)

def load_categories(conn, data):
    """
    Populate category tables from data structure.
    
    data = {
        "main": {
            "Category Name": {
                "subs": {
                    "Sub Name": {
                        "subsubs": {
                            "Subsub Name": True
                        }
                    }
                }
            }
        }
    }
    """
    cur = conn.cursor()
    
    mains_inserted = 0
    subs_inserted = 0
    subsubs_inserted = 0
    
    main_categories = data.get('main', {})
    
    for main_order, (main_name, main_data) in enumerate(main_categories.items()):
        # Insert main category
        try:
            cur.execute("""
                INSERT OR REPLACE INTO category_main (name, type, display_order)
                VALUES (?, 'Expense', ?)
            """, (main_name, main_order))
            main_id = cur.lastrowid
            mains_inserted += 1
            print(f"✓ Main: {main_name} (id={main_id})")
        except sqlite3.IntegrityError as e:
            print(f"  (Main '{main_name}' already exists, updating...)")
            cur.execute("SELECT id FROM category_main WHERE name = ?", (main_name,))
            row = cur.fetchone()
            main_id = row[0] if row else None
            if not main_id:
                print(f"  ERROR: Could not insert or find main category '{main_name}'", file=sys.stderr)
                continue
        
        # Insert sub categories
        subs = main_data.get('subs', {})
        for sub_order, (sub_name, sub_data) in enumerate(subs.items()):
            try:
                cur.execute("""
                    INSERT OR REPLACE INTO category_sub (main_id, name, display_order)
                    VALUES (?, ?, ?)
                """, (main_id, sub_name, sub_order))
                sub_id = cur.lastrowid
                subs_inserted += 1
                print(f"  ├─ Sub: {sub_name} (id={sub_id})")
            except sqlite3.IntegrityError:
                cur.execute("SELECT id FROM category_sub WHERE main_id = ? AND name = ?",
                           (main_id, sub_name))
                row = cur.fetchone()
                sub_id = row[0] if row else None
                if not sub_id:
                    print(f"    ERROR: Could not insert sub '{sub_name}'", file=sys.stderr)
                    continue
            
            # Insert subsub categories
            subsubs = sub_data.get('subsubs', {})
            for subsub_order, subsub_name in enumerate(subsubs.keys()):
                try:
                    cur.execute("""
                        INSERT OR REPLACE INTO category_subsub (sub_id, name, display_order)
                        VALUES (?, ?, ?)
                    """, (sub_id, subsub_name, subsub_order))
                    subsub_id = cur.lastrowid
                    subsubs_inserted += 1
                    print(f"    └─ Subsub: {subsub_name} (id={subsub_id})")
                except sqlite3.IntegrityError:
                    print(f"      ('{subsub_name}' already exists, skipped)")
    
    conn.commit()
    print(f"\n=== Summary ===")
    print(f"Mains inserted:    {mains_inserted}")
    print(f"Subs inserted:     {subs_inserted}")
    print(f"Subsubs inserted:  {subsubs_inserted}")

if __name__ == '__main__':
    print("Loading multi-level categories...\n")
    
    conn = connect_db()
    data = load_categories_json()
    load_categories(conn, data)
    conn.close()
    
    print("\n✓ Done. Categories loaded into database.")
