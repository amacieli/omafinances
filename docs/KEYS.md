# OmaFinances keys

**Source of truth:** `qml/keys/Keymap.js`. Every binding is registered once there. The help overlay (`?`) is generated from it at runtime. This file is written by hand to match it; when a key changes, change `Keymap.js` and this file in the same commit.

Scopes: a focused pane's keys are checked before `Anywhere`, so a pane can shadow a global key. Overlays (pickers, typed input, help) take keys only while open.

### Anywhere

| Key | Action |
| --- | --- |
| `?` | Keyboard help |
| `f` | Filter menu: account, category, date, amount, money, text |
| `s` | Sort picker (choosing the current column reverses it) |
| `1` | Sort by date (again: reverse) |
| `2` | Sort by amount (again: reverse) |
| `3` | Sort by payee (again: reverse) |
| `4` | Sort by account (again: reverse) |
| `5` | Sort by category (again: reverse) |
| `6` | Sort by notes (again: reverse) |
| `b` | Show or hide the sidebar |
| `Tab` | Focus next pane: sidebar, table, detail |
| `Shift-Tab` | Focus previous pane |
| `Left` | Move focus to the pane on the left (sidebar; opens it if hidden) |
| `Right` | Move focus to the pane on the right (table, then detail) |
| `i` | Show or hide the detail pane |
| `x` | Clear all filters |
| `X` | Clear the newest filter chip |
| `r` | Refresh from Lunch Money, then reload |
| `m` | Category manager (full pane; see below) |
| `D` | Dashboard and insights (monthly and category totals; see below) |
| `P` | Reports: scheduled email reports (see below) |
| `R` | Review queue: uncategorized rows with numbered suggestions (see below) |
| `u` | Undo the last change (categorizing, category edits); toast says what was undone |
| `Ctrl-r` | Redo the change that was undone |
| `/` | Slash search bar (type terms; `:` starts a command; again: reopens it with the active query) |
| `v` | Saved views (Enter applies, `d` deletes) |
| `q` | Quit |

### Transactions (focus: table)

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down one row |
| `k` / `Up` | Move up one row |
| `g` | First row |
| `G` | Last row (loads the remaining pages first) |
| `Ctrl-d` | Half page down |
| `Ctrl-u` | Half page up |
| `PgDown` | Page down (a full screen) |
| `PgUp` | Page up (a full screen) |
| `Home` / `End` | First / last row (same as `g` / `G`) |
| `Space` | Mark or unmark the cursor row, then move down |
| `V` | Start a visual range from the cursor (`j`/`k` extend it); `Esc` leaves visual mode and keeps the marks |
| `Ctrl-a` | Mark every row matching the filters (loads all matching pages first, capped at 5000) |
| `Enter` / `e` | Open the note dialog (NOTES column) for the marked rows, else the cursor row. It shows the transaction, prefills its note (empty when several rows are marked; it replaces all their notes), and counts characters out of 255; `u` undoes the save |
| `c` | Categorize: the marked rows, else the cursor row. `Uncategorized` clears; `+ New category…` takes a path like `Home/Pets/Dogs` |
| `C` | Categorize with preview: pick a category, then review clusters (similar payees, recurring, amount bands) before anything is written. Same rows as `c` (up to 500 marked); `y` applies in one batch, `u` undoes it |
| `Esc` | Clear marks first (ends visual mode first if it is on). Esc never quits; use `q`. |

### Category manager (`m`; overlay, keys work inside it)

| Key | Action |
| --- | --- |
| `j` / `Down`, `k` / `Up` | Move the cursor |
| `a` | Add a child of the cursor node |
| `A` | Add a sibling after the cursor node |
| `r` | Rename (prompt prefilled) |
| `H` | Outdent: move under the parent's parent, right after the parent |
| `L` | Indent: become the last child of the previous sibling |
| `J` / `K` | Reorder among siblings |
| `e` | Cycle kind: expense, income, transfer |
| `d` | Delete: confirm; transactions go to the chosen category (default Uncategorized); children move up |
| `M` | Merge into…: picker that excludes the node and its descendants, then confirm |
| `Enter` | Filter the table by this category and close |
| `Esc` | Close |

The header shows `max depth N`; a server refusal (e.g. too deep) shows in the status line.

### Categorize with preview (`C`; overlay, keys work inside it)

The header reads `Will categorize N of M uncategorized · K conflicts skipped · $T`. Conflict clusters (rows already in a different category) are shown in the warning color and skipped unless overwrite is on; same-category clusters are dimmed and write nothing. Each collapsed cluster shows up to three exemplars.

| Key | Action |
| --- | --- |
| `j` / `Down`, `k` / `Up` | Move the cursor (`PgDown` / `PgUp`: ten lines) |
| `Space` | Select or deselect the cluster under the cursor |
| `a` / `n` | Select every cluster / deselect every cluster |
| `Enter` | Expand or collapse a cluster's rows; on `… N more` load the next 50 |
| `[` / `]` | Threshold down / up by 0.05 (shown as a percent, 50%–100%); re-queries, keeping each cluster's selection by label |
| `o` | Overwrite: also take rows already in another category (re-queries) |
| `r` | Also make this a rule (saved in the same batch; undone with it) |
| `y` | Apply the selection in one batch; the toast says `u to undo` |
| `Esc` | Cancel: nothing is written |

Keys for the picker step are the usual ones; the preview only appears after a category is chosen.

### Dashboard and insights (`D`; overlay, keys work inside it)

Tabs: Dashboard (monthly out and in bars, category bars with share of total out, top payees, and a note on the transfers left out), Recurring, Anomalies, Transfers. Totals exclude transfer rows unless `t` includes them.

| Key | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | Next / previous tab |
| `j` / `Down`, `k` / `Up` | Move down / up the list (on the Dashboard tab: scroll) |
| `Enter` | Jump the table to that series, anomaly or transfer pair (the query is `p:"name"` for series, `id:N` for an anomaly, `id:N,M` for a pair), then close |
| `t` | Include or exclude transfers in the Dashboard totals |
| `Esc` | Close |

### Review queue (`R`; overlay, keys work inside it)

One entry per payee group (normalized payee + sign, uncategorized rows only). Each group shows its payee, row count, total, date span, the pending category (`none: press 1/2/3 or c` until one is chosen), its numbered suggestion chips, and `Will categorize N of M`. The header says which mode the queue is in: `Suggestions: top-3 accuracy ...` once the gate is open, or `Bootstrap: biggest payees first` with what unlocks suggestions. Suggest mode sorts by top-option confidence; bootstrap sorts by group size.

Pressing `1`/`2`/`3` or `c` only chooses the group's pending category. Nothing is written until `y`, which categorizes the ticked rows (one undoable request; `u` undoes it) and moves on to the next group. `y` with no ticked rows or no pending category shows a message and sends nothing. `Enter` opens the group's rows, paged by 50, where `Space` ticks or unticks single rows (all are ticked by default).

| Key | Action |
| --- | --- |
| `j` / `Down`, `k` / `Up` | Move between groups (and their rows when expanded) |
| `1` / `2` / `3` | Set the group's pending category to that suggestion (applied by `y`) |
| `c` | Choose the pending category with the category picker |
| `C` | Full categorize preview for the group's representative row |
| `y` | Categorize the ticked rows of the group as its pending category (`u` undoes it) |
| `Space` | Tick or untick this row (on the group header: all rows of the group) |
| `a` / `n` | Tick every row of the group / untick every row of the group |
| `Enter` | Show or hide the group's rows; on "more", load the next 50 |
| `s` | Skip: go to the next group (it stays in the queue) |
| `Backspace` | Previous group |
| `u` | Undo the last change |
| `Esc` | Collapse expanded groups; with none expanded, leave the queue |

### Sidebar (focus: sidebar, Tab)

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down (headers are skipped) |
| `k` / `Up` | Move up |
| `Enter` | Filter by this account or category (replaces that facet) |
| `PgDown` / `PgUp` | Move down / up a page |
| `Space` | Toggle this account or category as a filter |
| `x` | Clear the account or category filter |
| `Enter` | On a saved view: apply it (replaces the search and sets its sort) |
| `d` | Delete the saved view under the cursor (asks first) |
| `Esc` | Back to the table |

### Detail pane (focus: detail, Tab)

| Key | Action |
| --- | --- |
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |
| `PgDown` / `PgUp` | Scroll down / up a page |
| `i` | Close the detail pane |
| `Esc` | Back to the table |
| `Ctrl-y` | Accept the suggested category for an uncategorized row (the ghost chip; `u` undoes it). Off with the reason shown when suggestions are disabled |

### Pickers: filter menu, sort, sub-pickers

| Key | Action |
| --- | --- |
| `Down` / `Ctrl-n` / `j` | Next choice (`j` only while the filter text is empty) |
| `Up` / `Ctrl-p` / `k` | Previous choice (`k` only while the filter text is empty) |
| `Enter` | Choose |
| `Esc` | Cancel (back to the filter menu from a sub-picker) |
| `Delete` / `d` | Delete the highlighted saved view (asks first; `d` only while the filter text is empty) |

### Search bar (focus: search, opened with `/`)

The bar owns its keys while it is open. Typing filters the table live once the query parses; a query that does not parse keeps the last good result and shows the error under the field.

| Key | Action |
| --- | --- |
| `Tab` | Accept the highlighted completion (replaces the token, adds a space) |
| `Shift-Tab` | Previous completion |
| `Up` / `Down` | With the completion list open: previous / next completion. Otherwise: older / newer pinned query (the draft comes back when you walk past the newest) |
| `Enter` | Pin the query to history and close the bar; the table gets focus. On a `:` command: run it |
| `Esc` | Cancel: restore the query that was active when the bar opened |
| `Ctrl-u` | Clear the field |
| `Ctrl-w` | Delete the word before the cursor |

### Typed input (amount, dates, text)

| Key | Action |
| --- | --- |
| `Enter` | Apply (empty clears; a bad value is shown inline and never sent to the backend) |
| `Esc` | Cancel |

### Note dialog (Enter or `e` on a transaction)

| Key | Action |
| --- | --- |
| `Enter` | Save the note (empty removes it; over 255 characters or a control character is shown inline and never sent) |
| `Esc` | Cancel without saving |
| `Ctrl-U` | Clear the field |

The Save and Cancel buttons also work with the mouse. The detail pane moved to `i` only.

### Search syntax

Terms are separated by spaces and ANDed together. A pinned query shows as the first chip (`/ ...`); `x` clears it with the other filters. It is ANDed with the sidebar and menu filters.

| Term | Meaning |
| --- | --- |
| `coffee` | Bare word: matches payee or notes |
| `"blue bottle"` | Quoted phrase, matched as text |
| `-amazon` | Negated: excludes the bare word |
| `p:` / `a:` / `c:` | Payee / account / category path (`c:home/maint`, `c:none` for uncategorized) |
| `n:` | Your notes: `n:vacation` (contains), `n:*` (has any note), `-n:*` (no note) |
| `d:` | Date: `2026-10`, `2026-10-01..2026-11-15`, `>2026-01-01`, presets (`today`, `yesterday`, `this-week`, `last-week`, `this-month`, `last-month`, `this-quarter`, `last-quarter`, `ytd`, `this-year`, `last-year`), relative `7d` `3m` `1y` `>30d` |
| `amount:` / bare `>50` / `10..50` | Amount (absolute value; a bare comparison or range works without `amount:`) |
| `is:` | `recurring` `pending` `transfer` `income` `expense` `uncat` `categorized` |
| `sort:` | `sort:-amount`, `sort:date`, `sort:notes`, and so on (`-` reverses) |
| `a,b` | Comma list inside a value means OR: `a:chase,amex` |

Examples:

- `p:coffee` — payee contains "coffee"
- `a:chase d:last-month` — Chase transactions from last month
- `c:home/maint -p:hardware` — home maintenance, excluding payees matching "hardware"
- `"blue bottle" d:2026-10..2026-11-15` — a quoted phrase in a date range
- `>50 is:expense sort:-amount` — expenses over 50, largest first
- `d:>2026-01-01 is:recurring` — recurring items since January 2026
- `c:none d:7d` — uncategorized, last seven days
- `a:chase,amex d:ytd n:reimburse` — two accounts, year to date, notes mention "reimburse"

Commands (the bar text starts with `:`): `:save <name>` saves the active query and sort as a view (empty bar: the active query), `:view <name>` applies a view (fuzzy name), `:views` opens the saved views list, `:delete <name>`, `:clear` clears all filters, `:help` opens this help.

### Journal and commands (search bar, `/` then `:`)

| Command | Action |
| --- | --- |
| `:undo` | Same as `u` |
| `:redo` | Same as `Ctrl-r` |
| `:journal` | Read-only list of the last 20 changes, newest first; undone ones are marked |

### This help

| Key | Action |
| --- | --- |
| `Esc` / `q` / `?` | Close |
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |

Mouse: clicking a row selects it; the wheel scrolls; scrolling near the end loads the next page of 200 rows.

### Reports (`P`; overlay, keys work inside it)

Header: `GET /mail/status` (`Sending as <sender>`, `Gmail not authorized: run omafinances gmail-auth`, or `client not configured: set [mail] client_secret_file`). `:reports` opens the list; `:report <name>` opens a new report named so, with the current table query. Error envelopes are shown verbatim; a 409 `not_authenticated` adds the hint to run `omafinances gmail-auth` in a terminal.

| Key | Action |
| --- | --- |
| `j` / `Down`, `k` / `Up` | Move in the list |
| `n` | New report (query = the current table query, recipients = the sender) |
| `Enter` | Edit |
| `p` | Preview (subject, period, row count, the text part; `j`/`k`/`PgUp`/`PgDown` scroll, HTML is not rendered) |
| `t` | Send a test to the sender |
| `e` | Enable / disable the schedule |
| `r` | Run history |
| `d` | Delete (then `y` / `n`) |
| `Esc` | Close |

Editor: `j`/`k` move between fields, `Enter` types a text field (name, query, time, recipients) or steps a choice, `h`/`l` (or `Left`/`Right`) step a choice (shape, period, schedule daily/weekly/monthly/quarterly, weekday, day 1-28 or last), `Space` toggles redact, `s` or `Ctrl-s` saves, `Esc` cancels. The next 3 runs of the schedule update live (`GET /schedule/preview`).
