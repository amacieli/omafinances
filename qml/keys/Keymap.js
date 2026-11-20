// The one keymap for OmaFinances. Every key the UI answers to is listed here, once.
//   - AppShell resolves a key press to an id with resolve(), then runs the handler for that id.
//   - The help overlay (`?`) is generated from BINDINGS, grouped by scope.
//   - docs/KEYS.md is written by hand to match this file. This file is the source of truth.
//
// Scopes: "global" (anywhere in the window), "table", "sidebar", "detail" (the focused pane;
// its scope is checked before "global", so a pane can shadow a global key), "search" (the slash
// search bar, which owns its keys while open), "picker" and "prompt" (the overlays, which get keys
// only while open), "help" (the help overlay).
//
// Token format (token()): a single printable character as typed ("j", "G", "?", "1"), Ctrl as
// "C-x" (lower case), and "Esc Tab S-Tab Enter Space Up Down Left Right Backspace Delete PgUp PgDown".

var SCOPE_TITLES = {
  global: "Anywhere",
  table: "Transactions (focus: table)",
  sidebar: "Sidebar (focus: sidebar, Tab)",
  detail: "Detail pane (focus: detail, Tab)",
  search: "Slash search bar (focus: the / bar; typing is a search)",
  picker: "Pickers: filter menu, sort, sub-pickers",
  prompt: "Typed input (amount, dates, text)",
  manager: "Category manager (m; keys work inside the overlay)",
  smart: "Categorize with preview (C; keys work inside the overlay)",
  help: "This help"
}

var SCOPE_ORDER = ["global", "table", "manager", "smart", "sidebar", "detail", "search", "picker", "prompt", "help"]

var BINDINGS = [
  // ---- anywhere
  { id: "help", scope: "global", keys: ["?"], label: "Keyboard help" },
  { id: "filter.menu", scope: "global", keys: ["f"], label: "Filter menu: account, category, date, amount, money, text" },
  { id: "sort.menu", scope: "global", keys: ["s"], label: "Sort picker (choosing the current column reverses it)" },
  { id: "sort.date", scope: "global", keys: ["1"], label: "Sort by date (again: reverse)" },
  { id: "sort.amount", scope: "global", keys: ["2"], label: "Sort by amount (again: reverse)" },
  { id: "sort.payee", scope: "global", keys: ["3"], label: "Sort by payee (again: reverse)" },
  { id: "sort.account", scope: "global", keys: ["4"], label: "Sort by account (again: reverse)" },
  { id: "sort.category", scope: "global", keys: ["5"], label: "Sort by category (again: reverse)" },
  { id: "sidebar.toggle", scope: "global", keys: ["b"], label: "Show or hide the sidebar" },
  { id: "focus.next", scope: "global", keys: ["Tab"], label: "Focus next pane: sidebar, table, detail" },
  { id: "focus.prev", scope: "global", keys: ["S-Tab"], label: "Focus previous pane" },
  { id: "pane.left", scope: "global", keys: ["Left"], label: "Move focus to the pane on the left (sidebar; opens it if hidden)" },
  { id: "pane.right", scope: "global", keys: ["Right"], label: "Move focus to the pane on the right (table, then detail)" },
  { id: "detail.toggle", scope: "global", keys: ["i", "Enter"], label: "Show or hide the detail pane" },
  { id: "filter.clearAll", scope: "global", keys: ["x"], label: "Clear all filters" },
  { id: "filter.clearLast", scope: "global", keys: ["X"], label: "Clear the newest filter chip" },
  { id: "reload", scope: "global", keys: ["r"], label: "Refresh from Lunch Money, then reload" },
  { id: "search.open", scope: "global", keys: ["/"], label: "Slash search bar: type terms, : starts a command (again: reopens it with the active query)" },
  { id: "views.open", scope: "global", keys: ["v"], label: "Saved views (Enter applies, d deletes)" },
  { id: "manager.open", scope: "global", keys: ["m"], label: "Category manager: tree, add, rename, move, delete, merge" },
  { id: "undo", scope: "global", keys: ["u"], label: "Undo the last change (categorizing, category edits)" },
  { id: "redo", scope: "global", keys: ["C-r"], label: "Redo" },
  { id: "quit", scope: "global", keys: ["q"], label: "Quit" },

  // ---- transaction table
  { id: "move.down", scope: "table", keys: ["j", "Down"], label: "Move down one row" },
  { id: "move.up", scope: "table", keys: ["k", "Up"], label: "Move up one row" },
  { id: "move.first", scope: "table", keys: ["g", "Home"], label: "First row" },
  { id: "move.last", scope: "table", keys: ["G", "End"], label: "Last row (loads the remaining pages first)" },
  { id: "page.down", scope: "table", keys: ["C-d"], label: "Half page down" },
  { id: "page.fulldown", scope: "table", keys: ["PgDown"], label: "Page down (a full screen)" },
  { id: "page.up", scope: "table", keys: ["C-u"], label: "Half page up" },
  { id: "page.fullup", scope: "table", keys: ["PgUp"], label: "Page up (a full screen)" },
  { id: "mark.toggle", scope: "table", keys: ["Space"], label: "Mark or unmark this row, then move down" },
  { id: "visual.toggle", scope: "table", keys: ["V"], label: "Visual range: start here; j/k extend it; V again ends it, keeping the marks" },
  { id: "mark.all", scope: "table", keys: ["C-a"], label: "Mark every row that matches the filters (loads all pages; cap 5000)" },
  { id: "marks.clear", scope: "table", keys: ["Esc"], label: "Esc: end visual mode (keeps marks); then clear the marks; with nothing marked, quit" },
  { id: "categorize", scope: "table", keys: ["c"], label: "Categorize the marked rows, or this row (category picker)" },
  { id: "categorizeSmart", scope: "table", keys: ["C"], label: "Categorize with preview: pick a category, then review clusters before anything is written" },

  // ---- category manager (full-pane overlay; AppShell runs the actions)
  { id: "mgr.down", scope: "manager", keys: ["j", "Down"], label: "Move down the tree" },
  { id: "mgr.up", scope: "manager", keys: ["k", "Up"], label: "Move up the tree" },
  { id: "mgr.add", scope: "manager", keys: ["a"], label: "Add a child under the cursor (empty tree: add a top-level category)" },
  { id: "mgr.addSibling", scope: "manager", keys: ["A"], label: "Add a sibling after the cursor" },
  { id: "mgr.rename", scope: "manager", keys: ["r"], label: "Rename (prefilled)" },
  { id: "mgr.outdent", scope: "manager", keys: ["H"], label: "Outdent: move under the parent's parent, right after the parent" },
  { id: "mgr.indent", scope: "manager", keys: ["L"], label: "Indent: become the last child of the previous sibling" },
  { id: "mgr.shiftUp", scope: "manager", keys: ["K"], label: "Move up among siblings" },
  { id: "mgr.shiftDown", scope: "manager", keys: ["J"], label: "Move down among siblings" },
  { id: "mgr.kind", scope: "manager", keys: ["e"], label: "Cycle kind: expense, income, transfer" },
  { id: "mgr.delete", scope: "manager", keys: ["d"], label: "Delete (asks; its transactions go to Uncategorized or a category you pick)" },
  { id: "mgr.merge", scope: "manager", keys: ["M"], label: "Merge into... (asks for the target and confirms)" },
  { id: "mgr.filter", scope: "manager", keys: ["Enter"], label: "Filter the transactions by this category and close" },
  { id: "mgr.close", scope: "manager", keys: ["Esc"], label: "Close the manager" },

  // ---- categorize with preview (full-pane overlay after C and a category pick; AppShell runs the actions)
  { id: "smart.down", scope: "smart", keys: ["j", "Down"], label: "Move down the list" },
  { id: "smart.up", scope: "smart", keys: ["k", "Up"], label: "Move up the list" },
  { id: "smart.pagedown", scope: "smart", keys: ["PgDown"], label: "Move down ten lines" },
  { id: "smart.pageup", scope: "smart", keys: ["PgUp"], label: "Move up ten lines" },
  { id: "smart.toggle", scope: "smart", keys: ["Space"], label: "Select or deselect the cluster under the cursor" },
  { id: "smart.all", scope: "smart", keys: ["a"], label: "Select every cluster" },
  { id: "smart.none", scope: "smart", keys: ["n"], label: "Deselect every cluster" },
  { id: "smart.expand", scope: "smart", keys: ["Enter"], label: "Expand or collapse a cluster's rows; on \"more\" load the next 50 rows" },
  { id: "smart.thresholdDown", scope: "smart", keys: ["["], label: "Lower the threshold by 5 points (re-queries; selection kept by cluster)" },
  { id: "smart.thresholdUp", scope: "smart", keys: ["]"], label: "Raise the threshold by 5 points (re-queries; selection kept by cluster)" },
  { id: "smart.overwrite", scope: "smart", keys: ["o"], label: "Overwrite: also take rows already in another category (re-queries)" },
  { id: "smart.rule", scope: "smart", keys: ["r"], label: "Also make this a rule (saved with the change; undone with it)" },
  { id: "smart.apply", scope: "smart", keys: ["y"], label: "Apply the selection in one batch (u undoes all of it)" },
  { id: "smart.cancel", scope: "smart", keys: ["Esc"], label: "Cancel: nothing is written" },

  // ---- sidebar
  { id: "move.down", scope: "sidebar", keys: ["j", "Down"], label: "Move down (headers are skipped)" },
  { id: "move.up", scope: "sidebar", keys: ["k", "Up"], label: "Move up" },
  { id: "side.pagedown", scope: "sidebar", keys: ["PgDown"], label: "Move down a page" },
  { id: "side.pageup", scope: "sidebar", keys: ["PgUp"], label: "Move up a page" },
  { id: "side.apply", scope: "sidebar", keys: ["Enter"], label: "Filter by this account or category (replaces that facet); a saved view replaces the search" },
  { id: "side.toggle", scope: "sidebar", keys: ["Space"], label: "Toggle this account or category as a filter; on a saved view: apply it" },
  { id: "view.delete", scope: "sidebar", keys: ["d"], label: "Delete the saved view under the cursor (asks first)" },
  { id: "filter.clearFacet", scope: "sidebar", keys: ["x"], label: "Clear the account or category filter" },
  { id: "focus.table", scope: "sidebar", keys: ["Esc"], label: "Back to the table" },

  // ---- detail pane
  { id: "detail.down", scope: "detail", keys: ["j", "Down"], label: "Scroll down" },
  { id: "detail.up", scope: "detail", keys: ["k", "Up"], label: "Scroll up" },
  { id: "detail.pagedown", scope: "detail", keys: ["PgDown"], label: "Scroll down a page" },
  { id: "detail.pageup", scope: "detail", keys: ["PgUp"], label: "Scroll up a page" },
  { id: "detail.close", scope: "detail", keys: ["i", "Enter"], label: "Close the detail pane" },
  { id: "focus.table", scope: "detail", keys: ["Esc"], label: "Back to the table" },

  // ---- slash search bar (typing goes to the bar; these are the keys it answers to)
  { id: "search.cancel", scope: "search", keys: ["Esc"], label: "Cancel: restore the query that was active when the bar opened" },
  { id: "search.submit", scope: "search", keys: ["Enter"], label: "Pin the query to history and close the bar (on a : command: run it)" },
  { id: "search.complete", scope: "search", keys: ["Tab"], label: "Accept the highlighted completion (adds a space after it)" },
  { id: "search.backTab", scope: "search", keys: ["S-Tab"], label: "Previous completion" },
  { id: "search.up", scope: "search", keys: ["Up"], label: "Previous completion; with no list open, an older pinned query" },
  { id: "search.down", scope: "search", keys: ["Down"], label: "Next completion; with no list open, a newer pinned query" },
  { id: "search.clear", scope: "search", keys: ["C-u"], label: "Clear the field (the table shows everything again)" },
  { id: "search.delWord", scope: "search", keys: ["C-w"], label: "Delete the word before the cursor" },

  // ---- pickers (filter menu, sort, sub-pickers, saved views); typing filters the list
  { id: "pick.down", scope: "picker", keys: ["Down", "C-n", "j"], label: "Next choice (j only while the filter is empty)" },
  { id: "pick.up", scope: "picker", keys: ["Up", "C-p", "k"], label: "Previous choice (k only while the filter is empty)" },
  { id: "pick.choose", scope: "picker", keys: ["Enter"], label: "Choose" },
  { id: "pick.cancel", scope: "picker", keys: ["Esc"], label: "Cancel (back to the filter menu from a sub-picker)" },
  { id: "pick.delete", scope: "picker", keys: ["Delete", "d"], label: "Delete the highlighted saved view (asks first; d only while the filter is empty)" },

  // ---- typed inputs (amount, date range, text)
  { id: "prompt.submit", scope: "prompt", keys: ["Enter"], label: "Apply (empty clears; a bad value is shown inline)" },
  { id: "prompt.cancel", scope: "prompt", keys: ["Esc"], label: "Cancel" },

  // ---- help overlay
  { id: "help.close", scope: "help", keys: ["Esc", "q", "?"], label: "Close" },
  { id: "help.down", scope: "help", keys: ["j", "Down"], label: "Scroll down" },
  { id: "help.up", scope: "help", keys: ["k", "Up"], label: "Scroll up" }
]

// Turns a key event into a token, or "" when the key means nothing to us.
function token(e) {
  var m = e.modifiers
  if (m & (Qt.AltModifier | Qt.MetaModifier)) return ""
  if (m & Qt.ControlModifier) {
    if (e.key >= Qt.Key_A && e.key <= Qt.Key_Z) return "C-" + String.fromCharCode(e.key + 32)
    if (e.key === Qt.Key_Left) return "C-Left"
    if (e.key === Qt.Key_Right) return "C-Right"
    return ""
  }
  switch (e.key) {
    case Qt.Key_Escape: return "Esc"
    case Qt.Key_Tab: return (m & Qt.ShiftModifier) ? "S-Tab" : "Tab"
    case Qt.Key_Backtab: return "S-Tab"
    case Qt.Key_Return:
    case Qt.Key_Enter: return "Enter"
    case Qt.Key_Space: return "Space"
    case Qt.Key_Up: return "Up"
    case Qt.Key_Down: return "Down"
    case Qt.Key_Left: return "Left"
    case Qt.Key_Right: return "Right"
    case Qt.Key_Backspace: return "Backspace"
    case Qt.Key_Delete: return "Delete"
    case Qt.Key_Home: return "Home"
    case Qt.Key_End: return "End"
    case Qt.Key_PageUp: return "PgUp"
    case Qt.Key_PageDown: return "PgDown"
  }
  return (e.text.length === 1 && e.text > " ") ? e.text : ""
}

// The id bound to token `tok` in the first of `scopes` that binds it, or "".
// Scopes are checked in order, so pass the focused pane first and "global" last.
function resolve(scopes, tok) {
  if (tok === "") return ""
  for (var s = 0; s < scopes.length; s++) {
    for (var i = 0; i < BINDINGS.length; i++) {
      var b = BINDINGS[i]
      if (b.scope === scopes[s] && b.keys.indexOf(tok) >= 0) return b.id
    }
  }
  return ""
}

// Display form of a token: "C-d" -> "Ctrl-d", "S-Tab" -> "Shift-Tab".
function pretty(k) {
  if (k.indexOf("C-") === 0) return "Ctrl-" + k.substring(2)
  if (k.indexOf("S-") === 0) return "Shift-" + k.substring(2)
  return k
}

// Groups for the help overlay: [{title, rows:[{keys, label}]}] in SCOPE_ORDER.
function groups() {
  var out = []
  for (var s = 0; s < SCOPE_ORDER.length; s++) {
    var rows = []
    for (var i = 0; i < BINDINGS.length; i++) {
      var b = BINDINGS[i]
      if (b.scope !== SCOPE_ORDER[s]) continue
      rows.push({ keys: b.keys.map(pretty).join("  /  "), label: b.label })
    }
    if (rows.length) out.push({ scope: SCOPE_ORDER[s], title: SCOPE_TITLES[SCOPE_ORDER[s]], rows: rows })
  }
  return out
}

// Markdown table body for docs/KEYS.md (used to check the hand-written file; not run by the app).
function markdown() {
  var lines = []
  var g = groups()
  for (var i = 0; i < g.length; i++) {
    lines.push("")
    lines.push("### " + g[i].title)
    lines.push("")
    lines.push("| Key | Action |")
    lines.push("| --- | --- |")
    for (var j = 0; j < g[i].rows.length; j++)
      lines.push("| " + g[i].rows[j].keys + " | " + g[i].rows[j].label + " |")
  }
  return lines.join("\n")
}
