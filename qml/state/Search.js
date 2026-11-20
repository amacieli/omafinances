// Slash search helpers: the search bar's text editing, UTF-8 byte <-> UTF-16 offsets (the backend
// counts bytes, QML counts UTF-16 units), history walking, the `:` command line, and the reshaping
// of a /search/parse reply. Pure functions only (no QML types), checked with node:
// tests/qml/search.test.js.

var COMMANDS = ["save", "view", "views", "delete", "clear", "help", "undo", "redo", "journal"]

var COMMAND_HELP = {
  save: "save the active query and sort as a view",
  view: "apply a saved view (fuzzy name)",
  views: "open the saved views list",
  delete: "delete a saved view by name",
  clear: "clear every filter and the search",
  help: "open the keyboard help",
  undo: "undo the last change (categorizing, category edits)",
  redo: "redo the change that was undone",
  journal: "list the last 20 changes, newest first"
}

var SORT_FIELDS = ["date", "amount", "payee", "account", "category"]

function clampIndex(s, i) {
  var n = Number(i)
  if (!isFinite(n) || n < 0) return 0
  return Math.min(Math.floor(n), s.length)
}

// UTF-8 byte length of the first `u16` UTF-16 units of `text`.
function byteOffset(text, u16) {
  var s = String(text === undefined || text === null ? "" : text)
  var end = clampIndex(s, u16)
  var n = 0
  for (var i = 0; i < end; i++) {
    var c = s.charCodeAt(i)
    if (c < 0x80) n += 1
    else if (c < 0x800) n += 2
    else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length) {
      // A surrogate pair is one 4-byte character. Cut between its halves: count nothing for it.
      if (i + 1 < end) { n += 4; i++ } else break
    } else n += 3
  }
  return n
}

// UTF-16 index for a UTF-8 byte offset. A byte inside a character maps to that character's start,
// or to its end when roundUp is true (use roundUp for the end of a span).
function u16Offset(text, bytes, roundUp) {
  var s = String(text === undefined || text === null ? "" : text)
  var want = Math.max(0, Number(bytes) || 0)
  var n = 0, i = 0
  while (i < s.length && n < want) {
    var c = s.charCodeAt(i), len = 3, units = 1
    if (c < 0x80) len = 1
    else if (c < 0x800) len = 2
    else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length) { len = 4; units = 2 }
    if (n + len > want) return roundUp ? i + units : i
    n += len
    i += units
  }
  return i
}

// Replaces [start, end) (UTF-16) with `insert`, then one trailing space (unless a space already
// follows). Returns {text, cursor} with the cursor just after that space.
function replaceRange(text, start, end, insert) {
  var s = String(text === undefined || text === null ? "" : text)
  var a = clampIndex(s, start)
  var b = Math.max(a, Math.min(Number(end) || 0, s.length))
  var after = s.substring(b)
  var sep = /^\s/.test(after) ? "" : " "
  var out = s.substring(0, a) + String(insert) + sep + after
  return { text: out, cursor: a + String(insert).length + 1 }
}

// Ctrl-w: removes the whitespace and the word before the cursor.
function deleteWordBack(text, cursor) {
  var s = String(text === undefined || text === null ? "" : text)
  var c = clampIndex(s, cursor)
  var i = c
  while (i > 0 && /\s/.test(s.charAt(i - 1))) i--
  while (i > 0 && !/\s/.test(s.charAt(i - 1))) i--
  return { text: s.substring(0, i) + s.substring(c), cursor: i }
}

// Up/Down through the pinned history (GET /history, newest first). `index` is -1 for the draft
// the user is typing; `draft` holds that draft while the history is being walked.
// dir < 0 is older (Up), dir > 0 is newer (Down). Returns {index, draft, text} or null when the
// walk cannot move.
function walkHistory(index, draft, rows, text, dir) {
  var list = rows || []
  if (dir < 0) {
    if (index + 1 >= list.length) return null
    return { index: index + 1, draft: index < 0 ? String(text) : draft, text: String(list[index + 1]) }
  }
  if (index < 0) return null
  var ni = index - 1
  return { index: ni, draft: draft, text: ni < 0 ? draft : String(list[ni]) }
}

function unquote(s) {
  var t = String(s || "").trim()
  if (t.length >= 2 && t.charAt(0) === '"' && t.charAt(t.length - 1) === '"') return t.substring(1, t.length - 1)
  return t
}

// A bar text that starts with ":" is a command line. Returns null when it is not one,
// {error} for a bad command, or {cmd, arg}.
function parseCommand(text) {
  var t = String(text === undefined || text === null ? "" : text)
  if (t.charAt(0) !== ":") return null
  var m = /^\s*(\S*)\s*([\s\S]*)$/.exec(t.substring(1))
  var name = m[1].toLowerCase()
  var arg = m[2].trim()
  if (name === "") return { error: "type a command after the colon: " + COMMANDS.map(function(c) { return ":" + c }).join("  ") }
  if (COMMANDS.indexOf(name) < 0) return { error: "unknown command :" + name + " (try :help)" }
  if ((name === "save" || name === "view" || name === "delete") && arg === "")
    return { error: "usage: :" + name + " <name>" }
  return { cmd: name, arg: unquote(arg) }
}

function startsWithCI(s, prefix) {
  return String(s).toLowerCase().indexOf(String(prefix).toLowerCase()) === 0
}

// Completions for a command line, as {start, end, items:[{insert, label, detail}]} in UTF-16
// offsets of the token under the cursor. null when the text is not a command line.
// viewNames: the names of the saved views (for :view and :delete).
function commandCompletions(text, cursor, viewNames) {
  var s = String(text === undefined || text === null ? "" : text)
  var c = clampIndex(s, cursor)
  if (s.charAt(0) !== ":" || c < 1) return null
  var before = s.substring(0, c)
  var end = c
  while (end < s.length && !/\s/.test(s.charAt(end))) end++
  var sp = before.search(/\s/)
  if (sp < 0) {
    var prefix = before.substring(1)
    var items = []
    for (var i = 0; i < COMMANDS.length; i++) {
      if (!startsWithCI(COMMANDS[i], prefix)) continue
      items.push({ insert: COMMANDS[i], label: ":" + COMMANDS[i], detail: COMMAND_HELP[COMMANDS[i]] })
    }
    return { start: 1, end: end, items: items }
  }
  var cmd = before.substring(1, sp).toLowerCase()
  if (cmd !== "view" && cmd !== "delete") return { start: c, end: c, items: [] }
  var argStart = sp
  while (argStart < c && /\s/.test(before.charAt(argStart))) argStart++
  var partial = before.substring(argStart).replace(/^"/, "").toLowerCase()
  var names = viewNames || []
  var first = [], second = []
  for (var j = 0; j < names.length; j++) {
    var n = String(names[j])
    var low = n.toLowerCase()
    if (partial === "" || low.indexOf(partial) === 0) first.push(n)
    else if (low.indexOf(partial) >= 0) second.push(n)
  }
  var list = first.concat(second).map(function(n) {
    var quoted = /\s/.test(n) ? '"' + n + '"' : n
    return { insert: quoted, label: n, detail: "" }
  })
  return { start: argStart, end: end, items: list }
}

// Turns a sort value ("-amount", "date", "amount:asc", or {key, dir}) into {key, dir}, or null.
// A leading "-" means descending; without it the direction is ascending.
function sortFromValue(v) {
  if (v && typeof v === "object") {
    if (SORT_FIELDS.indexOf(String(v.key)) < 0) return null
    return { key: String(v.key), dir: v.dir === "desc" ? "desc" : "asc" }
  }
  var t = String(v === undefined || v === null ? "" : v).trim().toLowerCase()
  if (t === "") return null
  var dir = "asc"
  if (t.charAt(0) === "-") { dir = "desc"; t = t.substring(1) }
  else if (t.charAt(0) === "+") t = t.substring(1)
  var m = /^(\w+)(?:[:\s]+(asc|desc))?$/.exec(t)
  if (!m || SORT_FIELDS.indexOf(m[1]) < 0) return null
  if (m[2]) dir = m[2]
  return { key: m[1], dir: dir }
}

// The sort as the query language and the views store it: "-amount" for descending.
function sortToValue(sort) {
  return (sort && sort.dir === "desc" ? "-" : "") + (sort ? sort.key : "date")
}

// Reshapes a /search/parse reply for the bar. Offsets are converted to UTF-16 against `text`
// (the text that was sent). Returns:
//   {ok, canonical, matched|null, sortValue|null (from a sort: term), error|null,
//    completions: {start, end, items}}
// error = {start, end, message} in UTF-16 units. A reply with no completions gets items [].
function fromParse(text, resp) {
  var s = String(text === undefined || text === null ? "" : text)
  var r = resp || {}
  var out = {
    ok: !!r.ok && !r.parse_error,
    canonical: String(r.canonical || ""),
    matched: typeof r.matched === "number" ? r.matched : null,
    sortValue: null,
    error: null,
    completions: { start: 0, end: 0, items: [] }
  }
  var terms = r.terms || []
  for (var i = 0; i < terms.length; i++) {
    if (terms[i] && terms[i].field === "sort") out.sortValue = terms[i].value
  }
  var pe = r.parse_error
  if (pe) {
    var pos = Number(pe.pos) || 0, len = Number(pe.len) || 0
    out.error = {
      start: u16Offset(s, pos, false),
      end: u16Offset(s, pos + len, true),
      message: String(pe.message || "invalid search")
    }
  }
  var c = r.completions
  if (c && c.items && c.items.length) {
    out.completions = {
      start: u16Offset(s, Number(c.start) || 0, false),
      end: u16Offset(s, Number(c.end) || 0, true),
      items: c.items.map(function(it) {
        return { insert: String(it.insert), label: String(it.label || it.insert), detail: String(it.detail || "") }
      })
    }
  }
  return out
}
