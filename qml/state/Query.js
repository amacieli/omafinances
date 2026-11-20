// Filter and sort state for the transaction table, and the parsing of typed filter input.
// Pure functions only (no QML types), so it can be checked with node. The filter object is the
// one place the table's filters live; buildQuery() is the only place they become a query string.
//
// Filter object fields (all strings, "" = not set):
//   account, accountLabel     account id (from /accounts) and its name
//   category, categoryLabel   category id, or "none" for uncategorized
//   from, to                  YYYY-MM-DD, inclusive
//   min, max                  amount bounds as sent to the backend (decimal strings)
//   amountLabel               the amount as typed, for the chip ("10..50", ">100", "<20")
//   flow                      "out" (money out) | "in" (money in) | ""
//   text                      contains-text
//   slash                     the active slash query, canonical form from /search/parse ("" = none)

function emptyFilter() {
  return {
    account: "", accountLabel: "",
    category: "", categoryLabel: "",
    from: "", to: "",
    min: "", max: "", amountLabel: "",
    flow: "",
    text: "",
    slash: ""
  }
}

// Sort columns in the order the sort picker and keys 1-5 show them.
var SORT_KEYS = [
  { key: "date", label: "Date", dir: "desc" },
  { key: "amount", label: "Amount", dir: "desc" },
  { key: "payee", label: "Payee", dir: "asc" },
  { key: "account", label: "Account", dir: "asc" },
  { key: "category", label: "Category", dir: "asc" }
]

function sortInfo(key) {
  for (var i = 0; i < SORT_KEYS.length; i++) if (SORT_KEYS[i].key === key) return SORT_KEYS[i]
  return SORT_KEYS[0]
}

// Choosing a column: the current column reverses direction; any other starts at its own default.
function nextSort(cur, key) {
  if (cur && cur.key === key) return { key: key, dir: cur.dir === "asc" ? "desc" : "asc" }
  return { key: key, dir: sortInfo(key).dir }
}

function arrow(sort) { return sort.dir === "asc" ? "\u2191" : "\u2193" }

function pad2(n) { return (n < 10 ? "0" : "") + n }

function ymd(d) { return d.getFullYear() + "-" + pad2(d.getMonth() + 1) + "-" + pad2(d.getDate()) }

function validDate(s) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(s)) return false
  var p = s.split("-").map(Number)
  var d = new Date(p[0], p[1] - 1, p[2])
  return d.getFullYear() === p[0] && d.getMonth() === p[1] - 1 && d.getDate() === p[2]
}

// Date-range presets relative to `today` (local date).
function presets(today) {
  var y = today.getFullYear(), m = today.getMonth(), d = today.getDate()
  return [
    { key: "month", label: "This month", from: ymd(new Date(y, m, 1)), to: ymd(today) },
    { key: "lastmonth", label: "Last month", from: ymd(new Date(y, m - 1, 1)), to: ymd(new Date(y, m, 0)) },
    { key: "last30", label: "Last 30 days", from: ymd(new Date(y, m, d - 29)), to: ymd(today) },
    { key: "ytd", label: "This year to date", from: ymd(new Date(y, 0, 1)), to: ymd(today) },
    { key: "lastyear", label: "Last year", from: ymd(new Date(y - 1, 0, 1)), to: ymd(new Date(y - 1, 11, 31)) }
  ]
}

// "YYYY-MM-DD..YYYY-MM-DD" (or one date for one day). "" clears. Returns {error} or {clear} or {from,to}.
function parseDateRange(text) {
  var t = String(text || "").replace(/\s+/g, "")
  if (t === "") return { clear: true }
  var parts = t.split("..")
  if (parts.length === 1) parts = [t, t]
  if (parts.length !== 2) return { error: "use YYYY-MM-DD..YYYY-MM-DD" }
  if (!validDate(parts[0]) || !validDate(parts[1])) return { error: "dates must be real YYYY-MM-DD days" }
  if (parts[0] > parts[1]) return { error: "the start date is after the end date" }
  return { from: parts[0], to: parts[1] }
}

var NUM = /^-?\d+(\.\d{1,2})?$/

function cents(n) { return (Math.round(Number(n) * 100) / 100).toFixed(2) }

// Amount: "10..50", "10..", "..50", ">100", "<20". "" clears. Strict bounds become the nearest
// cent (">100" is min 100.01, "<20" is max 19.99) so the backend can stay inclusive.
function parseAmount(text) {
  var raw = String(text || "").trim()
  var t = raw.replace(/\s+/g, "")
  if (t === "") return { clear: true }
  if (t.charAt(0) === ">" || t.charAt(0) === "<") {
    var num = t.substring(1)
    if (!NUM.test(num)) return { error: "after > or < give a number, e.g. >100" }
    return t.charAt(0) === ">"
      ? { min: cents(Number(num) + 0.01), max: "", label: raw }
      : { min: "", max: cents(Number(num) - 0.01), label: raw }
  }
  var i = t.indexOf("..")
  if (i < 0) return { error: "use 10..50, >100 or <20" }
  var a = t.substring(0, i), b = t.substring(i + 2)
  if (a === "" && b === "") return { error: "give a range such as 10..50" }
  if (a !== "" && !NUM.test(a)) return { error: "\"" + a + "\" is not an amount" }
  if (b !== "" && !NUM.test(b)) return { error: "\"" + b + "\" is not an amount" }
  if (a !== "" && b !== "" && Number(a) > Number(b)) return { error: "the minimum is above the maximum" }
  return { min: a === "" ? "" : cents(a), max: b === "" ? "" : cents(b), label: raw }
}

// Contains-text: trimmed, at most 120 characters. "" clears.
function parseText(text) {
  var t = String(text || "").replace(/\s+/g, " ").trim()
  if (t === "") return { clear: true }
  if (t.length > 120) return { error: "keep it under 120 characters" }
  return { text: t }
}

// Filter chips in display order. Each has an id used by clearChip().
function chips(f) {
  var out = []
  if (f.slash) out.push({ id: "search", label: f.slash, kind: "search" })
  if (f.account) out.push({ id: "account", label: "account: " + (f.accountLabel || f.account) })
  if (f.category) out.push({ id: "category", label: "category: " + (f.category === "none" ? "uncategorized" : (f.categoryLabel || f.category)) })
  if (f.from || f.to) out.push({ id: "date", label: "date: " + (f.from === f.to ? f.from : f.from + ".." + f.to) })
  if (f.min !== "" || f.max !== "") out.push({ id: "amount", label: "amount: " + f.amountLabel })
  if (f.flow) out.push({ id: "flow", label: "money " + (f.flow === "out" ? "out" : "in") })
  if (f.text) out.push({ id: "text", label: "text: \"" + f.text + "\"" })
  return out
}

function isActive(f) { return chips(f).length > 0 }

// A copy of the filter with one chip cleared.
function clearChip(f, id) {
  var n = Object.assign({}, f)
  if (id === "account") { n.account = ""; n.accountLabel = "" }
  else if (id === "category") { n.category = ""; n.categoryLabel = "" }
  else if (id === "date") { n.from = ""; n.to = "" }
  else if (id === "amount") { n.min = ""; n.max = ""; n.amountLabel = "" }
  else if (id === "flow") n.flow = ""
  else if (id === "text") n.text = ""
  else if (id === "search") n.slash = ""
  return n
}

// Query string for GET /transactions. Empty filter values are left out.
function buildQuery(f, sort, offset, limit) {
  var p = ["sort=" + encodeURIComponent(sort.key), "dir=" + encodeURIComponent(sort.dir)]
  if (f.account) p.push("account=" + encodeURIComponent(f.account))
  if (f.category) p.push("category=" + encodeURIComponent(f.category))
  if (f.from) p.push("from=" + encodeURIComponent(f.from))
  if (f.to) p.push("to=" + encodeURIComponent(f.to))
  if (f.min !== "") p.push("min=" + encodeURIComponent(f.min))
  if (f.max !== "") p.push("max=" + encodeURIComponent(f.max))
  if (f.flow) p.push("flow=" + encodeURIComponent(f.flow))
  if (f.text) p.push("q=" + encodeURIComponent(f.text))
  if (f.slash) p.push("s=" + encodeURIComponent(f.slash))
  p.push("limit=" + limit)
  p.push("offset=" + offset)
  return "/transactions?" + p.join("&")
}

// Table amount: "1,234.56". Positive = money out, negative = money in (both shown as typed).
// Non-USD currencies get their code appended; USD stays unmarked.
function fmtAmountRow(a, currency) {
  var n = Number(a)
  if (!isFinite(n)) return ""
  var parts = Math.abs(n).toFixed(2).split(".")
  var s = (n < 0 ? "-" : "") + parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ",") + "." + parts[1]
  if (currency && currency !== "USD") s += " " + currency
  return s
}

// "12,345" for counts (no decimals).
function fmtCount(n) {
  var v = Math.round(Number(n) || 0)
  return String(v).replace(/\B(?=(\d{3})+(?!\d))/g, ",")
}

// "$1,234.56" (USD is the unmarked currency in this app, so the summary uses a plain $).
function fmtMoney(n) {
  var v = Number(n) || 0
  var parts = Math.abs(v).toFixed(2).split(".")
  return (v < 0 ? "-$" : "$") + parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ",") + "." + parts[1]
}
