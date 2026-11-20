// Category tree logic for the category manager and the categorize picker. Pure functions (no QML
// types), so node can check them (tests/qml/categories.test.js).
//
// Rows are the GET /categories rows, in tree order:
//   {id, parent_id, name, kind, depth, count, count_subtree, sum_out_subtree}
// parent_id null is the top level. Ids are numbers; the helpers normalize them so a string id
// still compares equal.

var PATH_SEP = " \u203a "   // "Home › Maintenance": how a full path is shown
var MAX_NAME = 80

function norm(v) {
  return v === undefined || v === null || v === "" ? null : Number(v)
}

function pidOf(r) { return norm(r.parent_id) }

function normName(s) { return String(s === undefined || s === null ? "" : s).trim().toLowerCase() }

function indexOfId(rows, id) {
  var want = Number(id)
  for (var i = 0; i < rows.length; i++) if (Number(rows[i].id) === want) return i
  return -1
}

function rowOf(rows, id) {
  var i = indexOfId(rows, id)
  return i < 0 ? null : rows[i]
}

// Direct children of pid (null = top level), in tree order.
function childrenOf(rows, pid) {
  var want = norm(pid)
  var out = []
  for (var i = 0; i < rows.length; i++) if (pidOf(rows[i]) === want) out.push(rows[i])
  return out
}

// Position of id among its siblings (0-based), or -1.
function siblingIndex(rows, id) {
  var r = rowOf(rows, id)
  if (!r) return -1
  var sibs = childrenOf(rows, pidOf(r))
  for (var i = 0; i < sibs.length; i++) if (Number(sibs[i].id) === Number(id)) return i
  return -1
}

// "Home › Maintenance" from the root down to id, or "" when id is unknown.
function pathOf(rows, id) {
  var names = []
  var cur = rowOf(rows, id)
  var guard = 0
  while (cur && guard++ < 100) {
    names.unshift(String(cur.name))
    cur = pidOf(cur) === null ? null : rowOf(rows, pidOf(cur))
  }
  return names.join(PATH_SEP)
}

// id first, then every descendant, in tree order.
function subtreeIds(rows, id) {
  var out = [Number(id)]
  var frontier = [Number(id)]
  while (frontier.length) {
    var next = []
    for (var i = 0; i < rows.length; i++) {
      var p = pidOf(rows[i])
      if (p !== null && frontier.indexOf(p) >= 0) {
        out.push(Number(rows[i].id))
        next.push(Number(rows[i].id))
      }
    }
    frontier = next
  }
  return out
}

// Outdent: under the parent's parent, right after the parent. null when already top level.
// position is the 0-based index among the new parent's children (the node lands there).
function outdentTarget(rows, id) {
  var r = rowOf(rows, id)
  if (!r) return null
  var p = pidOf(r)
  if (p === null) return null
  var parent = rowOf(rows, p)
  if (!parent) return null
  return { parent_id: pidOf(parent), position: siblingIndex(rows, p) + 1 }
}

// Indent: become the last child of the previous sibling. null when there is no previous sibling.
function indentTarget(rows, id) {
  var i = siblingIndex(rows, id)
  if (i <= 0) return null
  var r = rowOf(rows, id)
  var prev = childrenOf(rows, pidOf(r))[i - 1]
  return { parent_id: Number(prev.id), position: childrenOf(rows, prev.id).length }
}

// Whether the node can swap with its neighbour in direction delta (-1 up, 1 down).
function canShift(rows, id, delta) {
  var r = rowOf(rows, id)
  if (!r) return false
  var sibs = childrenOf(rows, pidOf(r))
  var j = siblingIndex(rows, id) + delta
  return j >= 0 && j < sibs.length
}

// Where the cursor goes after id is deleted (rows = the list before the delete): its first child
// (the children move up into its place), else the row after it, else the row before it.
function neighbourAfterDelete(rows, id) {
  var i = indexOfId(rows, id)
  if (i < 0) return null
  var kids = childrenOf(rows, id)
  if (kids.length) return Number(kids[0].id)
  if (i + 1 < rows.length) return Number(rows[i + 1].id)
  if (i > 0) return Number(rows[i - 1].id)
  return null
}

// Cursor index after a reload: the row with that id if it is still there, else the fallback
// index clamped to the list. -1 for an empty list.
function restoreIndex(rows, id, fallback) {
  if (rows.length === 0) return -1
  if (id !== null && id !== undefined && id >= 0) {
    var i = indexOfId(rows, id)
    if (i >= 0) return i
  }
  return Math.max(0, Math.min(rows.length - 1, Number(fallback) || 0))
}

// "Home/Pets/Dogs" -> {segments: ["Home", "Pets", "Dogs"]}. Names are trimmed; an empty name
// (a leading, trailing or doubled slash) is rejected. Returns {error} or {segments}.
function parsePath(text) {
  var t = String(text === undefined || text === null ? "" : text)
  if (t.trim() === "") return { error: "type a path such as Home/Pets/Dogs" }
  var parts = t.split("/")
  var segs = []
  for (var i = 0; i < parts.length; i++) {
    var s = parts[i].trim()
    if (s === "") return { error: "empty name in the path: check the slashes (Home/Pets/Dogs)" }
    if (s.length > MAX_NAME) return { error: "a name is at most " + MAX_NAME + " characters" }
    segs.push(s)
  }
  return { segments: segs }
}

// A single category name (add, rename): trimmed, not empty, no slash. Returns {error} or {name}.
function parseName(text) {
  var s = String(text === undefined || text === null ? "" : text).trim()
  if (s === "") return { error: "type a name" }
  if (s.indexOf("/") >= 0) return { error: "a name cannot contain /: add levels one at a time" }
  if (s.length > MAX_NAME) return { error: "a name is at most " + MAX_NAME + " characters" }
  return { name: s }
}

// Walks the segments down the tree (names match case-insensitively, like the server).
// Returns {id, parentId, missing}: when the whole path exists, id is its node and missing is [];
// otherwise id is null, parentId is the deepest existing node (null = top level) and missing are
// the names still to create, in order.
function resolvePath(rows, segs) {
  var cur = null
  for (var i = 0; i < segs.length; i++) {
    var want = normName(segs[i])
    var sibs = childrenOf(rows, cur)
    var hit = null
    for (var j = 0; j < sibs.length; j++) {
      if (normName(sibs[j].name) === want) { hit = sibs[j]; break }
    }
    if (!hit) return { id: null, parentId: cur, missing: segs.slice(i) }
    cur = Number(hit.id)
  }
  return { id: cur, parentId: null, missing: [] }
}

function fmtN(n) {
  return String(Math.round(Number(n) || 0)).replace(/\B(?=(\d{3})+(?!\d))/g, ",")
}

// Entries for the categorize / reassign / merge picker (the generic Picker's {key,label,detail,match}).
//   opts.uncategorized  adds "Uncategorized" (key "none"), counted as opts.uncatCount
//   opts.newEntry       adds "+ New category..." (key "new")
//   opts.exclude        category ids left out (a node and its subtree, for delete and merge)
// Tree entries are indented by depth; detail is the full path and the subtree count; match is the
// full path, so typing "hm" finds "Home › Maintenance".
function pickerEntries(rows, opts) {
  opts = opts || {}
  var exclude = (opts.exclude || []).map(Number)
  var out = []
  if (opts.uncategorized) {
    out.push({ key: "none", label: "Uncategorized", detail: fmtN(opts.uncatCount) + " txns", match: "Uncategorized" })
  }
  if (opts.newEntry) {
    out.push({ key: "new", label: "+ New category\u2026", detail: "type a path, e.g. Home/Pets/Dogs", match: "New category" })
  }
  for (var i = 0; i < rows.length; i++) {
    var r = rows[i]
    if (exclude.indexOf(Number(r.id)) >= 0) continue
    var indent = ""
    for (var d = 0; d < (Number(r.depth) || 0); d++) indent += "   "
    var path = pathOf(rows, r.id)
    out.push({ key: String(r.id), label: indent + String(r.name),
               detail: path + "  \u00b7  " + fmtN(r.count_subtree) + " txns", match: path })
  }
  return out
}

// Kind cycle for the manager's e key: expense -> income -> transfer -> expense.
function kindNext(kind) {
  if (kind === "expense") return "income"
  if (kind === "income") return "transfer"
  return "expense"
}

// Journal rows (GET /journal) for the read-only picker, newest first.
function journalEntries(journal) {
  var list = journal.slice().sort(function(a, b) { return Number(b.id) - Number(a.id) })
  return list.map(function(r) {
    var parts = [String(r.created_at || "").replace("T", " ").substring(0, 16), String(r.kind || "")]
    if (r.undone) parts.push("undone")
    if (r.superseded) parts.push("superseded")
    return { key: String(r.id), label: String(r.label || ""), detail: parts.filter(function(p) { return p !== "" }).join("  \u00b7  ") }
  })
}
