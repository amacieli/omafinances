# Finances Tracker — agent working notes

Weekly Lunch Money → **encrypted SQLCipher** SQLite sync, used for tax/finance questions. Not a git repo.
Queries: use the `finances-tracker-query` skill; sync/maintenance: the `finances-tracker` skill. Load them instead of re-deriving the access method.

## Files
- `sync.py` — pulls unseen transactions + accounts and upserts into `finances.db`. Catches up automatically from the day after the last successful sync.
  Runs weekly via Hermes cron (`finances-sync`, Sundays 00:00, working dir this folder). Log: `sync.log`.
- `schema.sql`, `migration_multilevel_categories.sql`, `categories.json`, `load_categories.py`, `query_categories.py` — three-level categories (Main / Sub / Subsub); see `MULTILEVEL_CATEGORIES_README.md`.
- `.env` (chmod 600, gitignored): `LUNCHMONEY_API_TOKEN`, `DB_PASSPHRASE`. `.venv/` has `sqlcipher3-binary`, `requests`, `python-dotenv` — run scripts with `.venv/bin/python`.

## Rules
- **Never print, log, echo or paste the API token or database passphrase** — not in chat, commands visible in output, scripts, or notes. Read them from `.env` inside the script.
- Never decrypt the database to a plaintext file. Query through SQLCipher with the passphrase read from the environment.
- Sync is read-only against Lunch Money; never write back to the user's Lunch Money account without being asked.
- Back up `finances.db` before any schema migration or bulk update; migrations must be re-runnable.
- If a sync fails, report the error from `sync.log` (without secrets) rather than retrying in a loop. Rate limits and a missed week are handled by the catch-up logic.
- Report amounts with currency and date range; for tax questions state which categories/accounts were included.
