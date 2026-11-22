# OmaFinances

Keyboard-first finance manager over the encrypted Lunch Money mirror (`finances.db`, SQLCipher),
maintained by `sync.py`. Rust backend + Quickshell (QML) UI, for Omarchy/Hyprland.

## Architecture

- **Backend** (`src/`, binary `omafinances`): opens the SQLCipher database, serves the data to the UI
  over loopback HTTP on **127.0.0.1:27183**. Every launch generates a new random token; the UI must
  present it, and the server also checks the `Host` header and rejects requests with an `Origin`
  header.
- **UI** (`qml/`, run by Quickshell with `QML_DIR`): talks only to the backend, never to the database.
- **Database**: SQLCipher file at `database.path` in `settings.toml` (default
  `/mnt/ai/projects/omafinances/finances.db`). The passphrase is not in any file the app writes:
  it lives in the Secret Service keyring (service `omafinances`, user `db-passphrase`).
- **Migrations**: the backend makes a backup of the database before any schema migration.
- **Shared database**: `sync.py` writes the same file. The app must not add FTS tables or triggers
  to it.

## API

Loopback JSON API for the UI. Every route needs the per-launch bearer token (401 without it). Paths
match exactly; the only patterns are `/transactions/<id>` and `/views/<id>`. Unknown path: 404.
Known path, wrong method: 405 (writes are `POST /refresh`, `POST /views`, `DELETE /views/<id>` and
`POST /history`; everything else is GET). Errors are `{"error": "..."}` and never include payees, amounts or tokens. If the
database is unavailable or not loaded yet, data routes return 503.

Transactions are held in memory (`src/cache.rs`) and loaded once at startup. Queries run against
that copy (`src/query.rs`). The cache does not follow `sync.py` on its own: call `POST /refresh`
after a sync.

- `GET /health`, `GET /status`, `GET /settings`: unchanged.
- `GET /transactions?sort=date|amount|payee|account|category&dir=asc|desc&account=<id>&category=<id>|none&from=YYYY-MM-DD&to=YYYY-MM-DD&min=<n>&max=<n>&flow=out|in&q=<text>&limit=1..500&offset=<n>`
  - Filters AND together. `category=<id>` includes that category and all its descendants.
    `category=none` means no local category.
  - `min`/`max` compare the absolute amount. `flow=out` is amount > 0 (spending); `flow=in` is
    amount < 0 (income, refunds).
  - `q` is a case-insensitive substring match over payee, original name and notes. Plain text only.
  - `s=<slash query>` (see below) ANDs with every filter above. An invalid query returns 400
    `{"error":"parse","parse_error":{"pos","len","message"}}`; a valid one adds
    `"search":{"canonical":"..."}` to the response. A `sort:` directive in `s` applies only when the
    request has neither `sort` nor `dir`; explicit parameters win.
  - Defaults: `sort=date&dir=desc`, `limit=100`, `offset=0`. Ties break by id descending.
  - Bad or duplicate parameters, and unknown parameter names, return 400. Nothing is silently
    ignored.
  - Response: `all` (rows in the cache), `total` (rows matching the filters, used for paging),
    `sum_out` and `sum_in` (over all matches, not only the page), and `rows`.
  - Each row: `id, date, payee, amount, currency, account_id, account, category` (local path such as
    `Home › Improvement › Paint`, or null), `category_id` (local id or null), `lm_category` (Lunch
    Money name or null), `pending`, `recurring`, `notes` (your own note, `transactions.user_notes`, or null; `sort=notes` is
    case-insensitive, no note first ascending).
- `GET /transactions/<id>`: the row plus `original_name, lm_notes (Lunch Money's notes), status, source, tags, recurring_id,
  external_id, created_at_lm, updated_at_lm`. 400 if `<id>` is not an integer, 404 if unknown.
- `GET /accounts`: `rows` ordered by name, each with `id, name, type, institution, mask, balance,
  currency, count, sum_out, nickname, formal_name`. `name` is the display name: nickname if set, else the
  account's display name, else its account name. `nickname` is your own name for the account (null when
  unset). `formal_name` is the account's own name, unchanged. `balance` is the latest snapshot, or null.
- `GET /categories`: `rows` in tree order (parent before children, siblings by sort then name), each
  with `id, parent_id, name, kind, depth, count, count_subtree, sum_out_subtree`, plus
  `uncategorized` (transactions with no local category).
- `POST /refresh`: reloads the cache from the database. Returns `{"ok": true, "transactions": N}`.
  On a failed reload the old cache keeps serving and the response is 500.

### Slash search

The text typed after `/` in the UI. It is parsed in Rust (`src/search/`), compiled to a predicate and
run over the in-memory cache (about 1 ms for 50k rows). Query text is private: it is never logged,
and error bodies carry only the parse message.

- Terms are separated by whitespace and ANDed. `-` before a term negates it. `"..."` groups a phrase
  (`\"` for a literal quote). A bare word is a case-insensitive substring over payee, original name
  and notes. `field:value` terms accept a comma list meaning OR (`p:amazon,costco`).
- `p:` (`payee`) payee or original name; `a:` (`acct`, `account`) account name; `n:` (`note`,
  `notes`) your notes (`n:*` = has a note, `-n:*` = none); `lm:` Lunch Money category name; `id:` exact ids.
- `c:` (`cat`, `category`) local category path. `c:home` is Home and all descendants; `c:home/maint`
  matches path segments by case-insensitive prefix (Home › Maintenance); `c:none` is no local
  category. Quote paths with spaces: `c:"home/sub 0"`. The completer inserts exactly this form.
- `d:` (`date`): `2026-10-05`, `2026-10`, `2026`, ranges `2026-10-01..2026-11-15` (open ends allowed),
  comparators `>2026-01-01 <2026-01-01 >= <=`, presets `today yesterday this-week last-week
  this-month last-month this-quarter last-quarter ytd this-year last-year` (weeks start Monday),
  relative `7d` (last 7 days, today included), `>30d` (newer than 30 days ago), `<30d`, `3m`, `1y`,
  `1w`. Relative dates resolve against the local date.
- `amount:` (`amt`), and a bare number with an operator or range (`>50 >=50 <20 10..50 100 =100`):
  compares the absolute amount. `$` and thousands commas are accepted (`>$1,200`). A comma followed by
  exactly three digits is a thousands separator, any other comma is OR (`amount:10,20`).
- `is:` `recurring pending transfer income expense uncat categorized`. `income` is amount < 0,
  `expense` is amount > 0, `transfer` is a Lunch Money category containing "transfer".
- `sort:date|amount|payee|account|category|notes`, `-` before the key for descending (`sort:-amount`). It is a
  directive, not a filter; once per query.
- Errors name the byte position and length and suggest a fix (`unknown field "pyee"; did you mean
  "p"?`). Rows whose stored date is not a valid date never match a `d:` term.

Routes:

- `GET /search/parse?q=<text>&pos=<cursor byte offset, default end>`: always 200. `ok`, `canonical`
  (null if invalid), `terms` (`text, neg, field, value`), `sort` (e.g. `"-amount"` or null),
  `parse_error` (null or `pos, len, message`), `matched` (rows matching, null if invalid or no
  database) and `completions: {start, end, items: [{insert, label, detail}]}` (at most 12). The UI
  replaces `text[start:end]` with `insert`. Completion covers field names, then values: payees
  (most frequent first), accounts, category paths, date presets, `is:` values, sort keys.
- `GET /views`: `rows` of `id, name, query, sort, position`, by position then name.
  `POST /views` with `{"name","query","sort"?}` upserts by name (case-insensitive) and returns the row.
  400 for an empty or over-60-character name, control characters, a non-parsing query or an unknown
  sort. `DELETE /views/<id>` returns `{"ok":true}`, 404 if unknown. Every write is journaled as a
  `change_batch` (kind `view`) with one `change_item` (entity `saved_view`, field `row`, old and new
  row as JSON) for the undo phase.
- `GET /history`: `rows` of query strings, newest first, at most 50. `POST /history` with `{"q"}`
  adds it, moving an older identical entry to the front; empty or unparsable queries are ignored
  (`"added": false`), more than 500 characters is 400. Stored as query text only in
  `<config dir>/search_history.json` (mode 0600, atomic replace).
- Request bodies (`POST /views`, `POST /history`) must be a JSON object of at most 64 KiB. No
  Content-Type is needed (the UI sends `text/plain`). Anything else is 400, over the limit 413.

Amounts follow the Lunch Money convention: positive is money out, negative is money in.

### Categories, categorize, undo (phase 3)

All mutations are POST with a JSON body, run in one SQLite transaction, write one `change_batch` (journal) and refresh the in-memory cache. Errors are `{"error":code,"message":text}` with 400 invalid, 404 unknown id, 409 clash/limit/conflict.

| Route | Body | Notes |
| --- | --- | --- |
| `POST /categories` | `{parent_id, name, kind?}` | depth limit `[categories] max_depth` (default 3) in settings.toml |
| `POST /categories/<id>/rename` | `{name}` | siblings unique, case-insensitive |
| `POST /categories/<id>/kind` | `{kind}` | expense / income / transfer |
| `POST /categories/<id>/move` | `{parent_id, position?}` | refuses cycles and subtree depth overflow |
| `POST /categories/<id>/shift` | `{delta:-1\|1}` | swap with neighbouring sibling |
| `POST /categories/<id>/delete` | `{reassign_to}` | its transactions move (or become uncategorized); children move up |
| `POST /categories/<id>/merge` | `{into}` | same-name children merge recursively |
| `POST /transactions/categorize` | `{ids, category_id}` | max 5000 ids; `null` uncategorizes |
| `PUT /transactions/<id>/notes` | `{notes}` | string up to 255 chars (trimmed; `""` or null clears); 400 too long, 404 unknown id; 200 `{ok, batch_id, label}` (`batch_id` null if unchanged) |
| `POST /transactions/notes` | `{ids, notes}` | one note on up to 5000 ids, one undo step; unknown id: 404, nothing changes |
| `PUT /accounts/<id>/nickname` | `{nickname}` | string up to 40 chars (trimmed; `""` or null clears); no control characters; 400 invalid, 404 unknown account; 200 `{ok, batch_id, label}` (`batch_id` null if unchanged). Undoable. Shown as the account's `name` everywhere (list, transaction rows, `a:` search) |
| `POST /undo`, `POST /redo` | none | 409 `nothing_to_undo` / `conflict` (data changed underneath) |
| `GET /journal?limit=` | | `{rows, can_undo, can_redo, next_undo_label, next_redo_label}` |

A new change supersedes (makes un-redoable) every undone batch. Saved-view writes are journaled too. `GET /settings` includes `max_depth`.

## Build, run, test

```sh
./build.sh                 # debug build -> target/debug/omafinances (or OMAFINANCES_BUILD_DIR)
./build.sh --release       # release build
./build.sh --clean --live  # clean, then install live (see below)
./build.sh --help
cargo test                 # unit tests
```

`--live` implies `--release`. It installs to `${OMAFINANCES_LIVE_DIR:-/opt/omafinances}`:

- `bin/omafinances` (installed by rename, so a running instance is not disturbed mid-write)
- `qml/` (copied from the project, without the Omarchy shell-kit links `Commons` and `Ui`)
- `share/omafinances.svg` (icon)
- `omafinances`: the launcher. If the window is already open, it focuses it instead of starting a
  second one.
- `~/.local/share/applications/OmaFinances.desktop`: the menu entry. It is written only when missing
  or different. Omarchy's Apps menu picks it up from there; no menu JSON is edited.

`--live` stops any running OmaFinances window and backend first (the live copy and the dev copies
under the build dir only; other apps are never matched) and restarts the app afterwards if it was
open. The build log and restart log go to the build dir.

Test hooks: `OMAFINANCES_SKIP_BUILD=1 OMAFINANCES_PREBUILT=<binary>` skips cargo and installs the
given binary. Use it with `OMAFINANCES_LIVE_DIR` and `XDG_DATA_HOME` pointing at scratch dirs to
rehearse the install without touching `/opt` or the real menu.

## Config directory

`${XDG_CONFIG_HOME:-~/.config}/omafinances/` (mode 0700):

- `settings.toml` (mode 0600), created with defaults on first run:
  - `[database] path` — the SQLCipher file.
  - `[ui] scale_factor` — 0.5 to 3.0, default 1.0.
  - `[mcp] enabled` — default `false`. Reserved; the MCP server only runs while the app is open.
- `build.env` (optional, not committed): `OMAFINANCES_BUILD_DIR=/path/to/build` tells `build.sh`
  where cargo writes its output.

No secrets are stored in either file.

## First run

1. `./build.sh --live`. The first run asks for sudo once to create `/opt/omafinances` owned by you.
   Later runs need no sudo.
2. Put the database passphrase in the keyring: `omafinances keyring-import`. This copies
   `DB_PASSPHRASE` from a `.env`-style file into the Secret Service keyring. If the keyring entry is
   missing, run this command, then start the app again.
3. Launch OmaFinances from the Omarchy Apps menu (or `/opt/omafinances/omafinances`).

## Security rules

- No secrets in logs, in error messages, or in the UI. Never print the passphrase or the Lunch Money
  token.
- Bound every SQL parameter; never build SQL by string concatenation from user input.
- Every HTTP request is checked against the per-launch token; the token is never written to disk.
- Never decrypt the database to a plaintext file, not even temporarily.
- Read-only against Lunch Money: the app never writes back to Lunch Money.
- Back up the database before any migration; migrations must be re-runnable.
- Nothing is written outside the database, the config dir, the live dir and the menu entry.
