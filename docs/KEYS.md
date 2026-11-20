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
| `b` | Show or hide the sidebar |
| `Tab` | Focus next pane: sidebar, table, detail |
| `Shift-Tab` | Focus previous pane |
| `Left` | Move focus to the pane on the left (sidebar; opens it if hidden) |
| `Right` | Move focus to the pane on the right (table, then detail) |
| `i` / `Enter` | Show or hide the detail pane |
| `x` | Clear all filters |
| `X` | Clear the newest filter chip |
| `r` | Refresh from Lunch Money, then reload |
| `/` | Slash search (next phase) |
| `q` | Quit |

### Transactions (focus: table)

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down one row |
| `k` / `Up` | Move up one row |
| `g` | First row |
| `G` | Last row (loads the remaining pages first) |
| `Ctrl-d` | Page down (half a screen) |
| `Ctrl-u` | Page up (half a screen) |
| `Esc` | Quit (from the table; elsewhere `Esc` returns to the table) |

### Sidebar (focus: sidebar, Tab)

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down (headers are skipped) |
| `k` / `Up` | Move up |
| `Enter` | Filter by this account or category (replaces that facet) |
| `Space` | Toggle this account or category as a filter |
| `x` | Clear the account or category filter |
| `Esc` | Back to the table |

### Detail pane (focus: detail, Tab)

| Key | Action |
| --- | --- |
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |
| `i` / `Enter` | Close the detail pane |
| `Esc` | Back to the table |

### Pickers: filter menu, sort, sub-pickers

| Key | Action |
| --- | --- |
| `Down` / `Ctrl-n` / `j` | Next choice (`j` only while the filter text is empty) |
| `Up` / `Ctrl-p` / `k` | Previous choice (`k` only while the filter text is empty) |
| `Enter` | Choose |
| `Esc` | Cancel (back to the filter menu from a sub-picker) |

### Typed input (amount, dates, text)

| Key | Action |
| --- | --- |
| `Enter` | Apply (empty clears; a bad value is shown inline and never sent to the backend) |
| `Esc` | Cancel |

### This help

| Key | Action |
| --- | --- |
| `Esc` / `q` / `?` | Close |
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |

Mouse: clicking a row selects it; the wheel scrolls; scrolling near the end loads the next page of 200 rows.
