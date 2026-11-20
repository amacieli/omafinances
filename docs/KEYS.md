# OmaFinances keys

This table is the **source of truth** for OmaFinances keybindings. When a key changes, change this file in the same commit as `qml/AppShell.qml` (`Keys.onPressed`).

| Key | Action |
| --- | --- |
| `j`, `Down` | Move the selection down one row |
| `k`, `Up` | Move the selection up one row |
| `g` | Jump to the first row |
| `G` | Jump to the last row (loads the remaining pages first, so it is the real last row) |
| `Ctrl-d` | Page down (half of the visible rows) |
| `Ctrl-u` | Page up (half of the visible rows) |
| `r` | Reload status and transactions from the backend |
| `q`, `Esc` | Quit |

Notes:

- Mouse: clicking a row selects it; the wheel scrolls. Scrolling near the end loads the next page of 200 rows.
- Text fields own the keyboard while focused. None exist yet; when one is added, its keys take precedence and this table gets a note.
- Only the transaction table is keyboard-driven in Phase 0. There are no mutating keys: the UI is read-only.
