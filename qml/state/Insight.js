// Phase 5 (intelligence and insights): pure helpers for the dashboard/insights overlay, the review
// queue and the detail pane's suggestion line. No QML here. tests/qml/insight.test.js covers them.
// AppShell owns every request and every piece of state; the views only render what these return.

var TABS = ["dashboard", "recurring", "anomalies", "transfers"]
var TAB_TITLES = { dashboard: "Dashboard", recurring: "Recurring", anomalies: "Anomalies", transfers: "Transfers" }
var REVIEW_PAGE = 50        // rows per GET /review request
var REVIEW_MAX = 200        // the route's limit ceiling
var OPTION_MAX = 3          // numbered chips in the review queue (keys 1..3)
var ACCURACY_MIN_LABELED = 30
var ACCURACY_MIN_TOP3 = 0.70
var PER_YEAR = { weekly: 52, biweekly: 26, monthly: 12, quarterly: 4, yearly: 1 }

// Keeps i inside 0..n-1 (0 when n is 0).
function clamp(i, n) {
  if (n <= 0) return 0
  return Math.max(0, Math.min(n - 1, Number(i) || 0))
}

// Cursor move by dir (+1/-1) inside a list of n rows, clamped (no wrap).
function moveIndex(i, dir, n) { return clamp(Number(i) + dir, n) }

// Tab cycling over TABS. dir is +1 (Tab) or -1 (Shift-Tab); wraps.
function nextTab(i, dir) {
  var n = TABS.length
  return (((Number(i) + dir) % n) + n) % n
}

function tabTitle(i) { return TAB_TITLES[TABS[i]] || "" }

// ---------------------------------------------------------------- money and numbers
function fmtN(n) {
  return String(Math.round(Math.abs(Number(n) || 0))).replace(/\B(?=(\d{3})+(?!\d))/g, ",")
}

// "$1,234" (whole dollars). Negative keeps its sign: "-$1,234".
function money(n) {
  var v = Number(n) || 0
  return (v < 0 ? "-$" : "$") + fmtN(Math.round(Math.abs(v)))
}

// "$1,234.56" for row amounts (cents).
function moneyCents(n) {
  var v = Number(n) || 0
  var parts = Math.abs(v).toFixed(2).split(".")
  return (v < 0 ? "-$" : "$") + parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ",") + "." + parts[1]
}

// Signed: "+$1,234" / "-$200" (net figures).
function signedMoney(n) {
  var v = Number(n) || 0
  var r = Math.round(Math.abs(v))
  if (r === 0) return "$0"
  return (v > 0 ? "+$" : "-$") + fmtN(r)
}

// Signed with cents: "+$1,234.50" / "-$200.00" (net figures in the views).
function signedMoneyCents(n) {
  var v = Number(n) || 0
  if (Math.abs(v) < 0.005) return "$0.00"
  return (v > 0 ? "+" : "") + moneyCents(v)
}

// A fraction 0..1 as "78%". Non-numbers and NaN read as 0%.
function pct(x) {
  var v = Number(x)
  if (!isFinite(v)) v = 0
  return Math.round(v * 100) + "%"
}

// part / total as a fraction; 0 when total is 0.
function share(part, total) {
  var t = Number(total) || 0
  if (t <= 0) return 0
  return Math.max(0, Math.min(1, (Number(part) || 0) / t))
}

// Pixel width of a bar: value scaled against max into maxPx. A positive value is at least 1px so
// it stays visible; zero (or no max) is 0.
function barWidth(value, max, maxPx) {
  var v = Number(value) || 0
  var m = Number(max) || 0
  if (v <= 0 || m <= 0 || maxPx <= 0) return 0
  var w = Math.round(v / m * maxPx)
  return Math.max(1, Math.min(maxPx, w))
}

// The largest value of a list of numbers (0 for none), reading the field `key` when given.
function maxOf(list, key) {
  var m = 0
  for (var i = 0; i < list.length; i++) {
    var v = Number(key ? list[i][key] : list[i]) || 0
    if (v > m) m = v
  }
  return m
}

// "2026-10" -> "Oct 2026". Anything else comes back unchanged.
function monthLabel(ym) {
  var names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
  var m = /^(\d{4})-(\d{2})$/.exec(String(ym || ""))
  if (!m) return String(ym || "")
  var mi = Number(m[2]) - 1
  if (mi < 0 || mi > 11) return String(ym)
  return names[mi] + " " + m[1]
}

// ---------------------------------------------------------------- dashboard
// Category rows with the bar width (of maxPx) and the share of the total out. Input is the
// /dashboard by_category array; totalOut is the dashboard totals.out.
function categoryBars(list, totalOut, maxPx) {
  var top = maxOf(list || [], "out")
  var out = []
  for (var i = 0; i < (list || []).length; i++) {
    var r = list[i]
    out.push({
      path: String(r.path || ""),
      category_id: r.category_id === undefined ? null : r.category_id,
      out: Number(r.out) || 0,
      count: Number(r.count) || 0,
      width: barWidth(r.out, top, maxPx),
      shareText: pct(share(r.out, totalOut)),
      label: money(r.out) + "  \u00b7  " + pct(share(r.out, totalOut)) + "  \u00b7  " + (Number(r.count) || 0) + " txns"
    })
  }
  return out
}

// Net sign for the monthly list: "+$1,234" / "-$200".
function monthNet(m) { return signedMoney(m.net) }

// The note under the dashboard: what the transfer exclusion removed (or that nothing was).
function transfersNote(excluded, include) {
  if (include) return "Transfers included (press t to exclude them again)."
  var ex = excluded || {}
  var n = Number(ex.pairs) || 0
  if (n <= 0) return "No transfer pairs found, so nothing is excluded. t includes transfers when there are some."
  return "Transfers excluded: " + n + (n === 1 ? " pair" : " pairs") + ", " + money(ex.total)
         + " moved between your accounts. t includes them."
}

// The dashboard's date span: "2025-12-01 to 2026-11-01".
function rangeText(d) {
  if (!d || !d.from) return ""
  return String(d.from) + " to " + String(d.to || "")
}

// ---------------------------------------------------------------- insights: series, anomalies, pairs
// Annualized cost of a recurring series: its median amount times its occurrences a year.
function annualCost(s) {
  var per = PER_YEAR[String(s.period || "")] || 0
  return Math.abs(Number(s.amount_median) || 0) * per
}

// "monthly  ·  14 charges  ·  median $15.49  ·  $186/yr  ·  next 2026-12-03"
function seriesLine(s) {
  var parts = [String(s.period || "?"), (Number(s.count) || 0) + " charges",
               "median " + moneyCents(s.amount_median)]
  var a = annualCost(s)
  if (a > 0) parts.push(money(a) + "/yr")
  if (s.next_expected) parts.push("next " + String(s.next_expected))
  return parts.join("  \u00b7  ")
}

// Sum of the annualized cost of the active series (what the list header states).
function activeAnnualTotal(list) {
  var t = 0
  for (var i = 0; i < (list || []).length; i++) if (list[i].active) t += annualCost(list[i])
  return t
}

// Anomaly kind as the list shows it.
function anomalyLabel(kind) {
  if (kind === "amount_spike") return "spike"
  if (kind === "duplicate") return "duplicate"
  if (kind === "new_payee") return "new payee"
  return String(kind || "")
}

// The search query that shows a payee's rows: p:"netflix". Quotes inside the payee are dropped,
// since the query language has no escape for them. "" when there is no payee text.
function payeeQuery(payee) {
  var clean = String(payee || "").replace(/["\\]/g, "").replace(/\s+/g, " ").trim()
  if (clean === "") return ""
  return 'p:"' + clean + '"'
}

// The search query for a set of exact transaction ids: id:12,34. "" for none.
function idsQuery(ids) {
  var out = []
  for (var i = 0; i < (ids || []).length; i++) {
    if (ids[i] === null || ids[i] === undefined || ids[i] === "") continue
    var v = Number(ids[i])
    if (isFinite(v) && Math.floor(v) === v && v >= 0) out.push(String(v))
  }
  return out.length === 0 ? "" : "id:" + out.join(",")
}

// Anomalies: the row's amount is shown as money out (positive), its detail is the backend's text.
function anomalyLine(a) {
  return anomalyLabel(a.kind) + "  \u00b7  " + String(a.date || "") + "  \u00b7  " + String(a.payee || "")
         + "  \u00b7  " + moneyCents(a.amount)
}

// Transfer pair: "Chase Checking -> Ally Savings  $500.00  1 day apart".
function pairLine(p) {
  var days = Number(p.days_apart) || 0
  return String(p.out_account || "?") + " \u2192 " + String(p.in_account || "?") + "   "
         + moneyCents(Math.abs(Number(p.amount) || 0)) + "   "
         + (days === 1 ? "1 day apart" : days + " days apart")
}

// ---------------------------------------------------------------- suggestions
// "Home › Groceries  82%  model" (confidence as a whole percent).
function optionText(o) {
  return String(o.path || "") + "  " + pct(o.confidence) + "  " + String(o.source || "")
}

// The first option of a suggestion row, or null.
function topOption(sugg) {
  if (!sugg || !sugg.options || sugg.options.length === 0) return null
  return sugg.options[0]
}

// Whether the accuracy gate is open, from a /suggest/accuracy body or null (not loaded).
function suggestionsEnabled(acc) {
  return !!(acc && acc.enabled === true)
}

// The line that says why suggestions are off, or "" when they are on.
function offReason(acc) {
  if (!acc) return "checking suggestions..."
  if (acc.enabled === true) return ""
  var r = acc.reason
  return r ? String(r) : "suggestions are off"
}

// The detail pane's line for an uncategorized row: the ghost chip text, or why there is none.
// sugg is the /suggest row for that id (or null), acc the /suggest/accuracy body (or null).
function detailSuggestLine(acc, sugg) {
  if (!acc) return "checking suggestions..."
  if (acc.enabled !== true) return "suggestions off: " + offReason(acc)
  var top = topOption(sugg)
  if (!top) return "no suggestion for this row (Ctrl-y needs one)"
  return "Ctrl-y accepts: " + String(top.path) + "  " + pct(top.confidence)
}

// Accuracy numbers as the header states them.
function accuracyText(a) {
  if (!a) return ""
  return "top-3 accuracy " + pct(a.top3) + " on " + (Number(a.labeled) || 0) + " labeled"
}

// The review header: mode, the reason and the counts (remaining and loaded count groups).
// info: {mode: "suggest"|"frequency", acc: /suggest/accuracy body or null, remaining, index, loaded}
// Returns {mode, detail, progress}. Texts follow the Phase 5 contract wording.
function reviewHeader(info) {
  var acc = info.acc || null
  var labeled = acc && acc.accuracy ? (Number(acc.accuracy.labeled) || 0) : 0
  var mode = ""
  var detail = ""
  if (info.mode === "suggest") {
    mode = "Suggestions: " + accuracyText(acc && acc.accuracy ? acc.accuracy : null)
  } else if (acc === null) {
    mode = "Bootstrap: biggest payees first"
    detail = "checking suggestion accuracy..."
  } else {
    mode = "Bootstrap: biggest payees first - suggestions unlock after "
           + ACCURACY_MIN_LABELED + " labeled rows and " + Math.round(ACCURACY_MIN_TOP3 * 100)
           + "% top-3 accuracy (you have " + labeled + ")"
    if (acc.accuracy && labeled >= ACCURACY_MIN_LABELED)
      detail = "top-3 is " + pct(acc.accuracy.top3) + " now"
  }
  return { mode: mode, detail: detail, progress: groupProgress(info.remaining, info.index, info.loaded) }
}

// The numbered option chips of a review row, at most OPTION_MAX.
function optionChips(options) {
  var out = []
  var list = options || []
  for (var i = 0; i < list.length && out.length < OPTION_MAX; i++) {
    var o = list[i]
    out.push({ n: out.length + 1, category_id: o.category_id, path: String(o.path || ""),
               confidence: Number(o.confidence) || 0, source: String(o.source || ""),
               text: String(o.path || "") + "   " + pct(o.confidence) + "   " + String(o.source || "") })
  }
  return out
}

// The option for key digit d ("1".."3"), or null when the row has no such option.
function optionFor(options, d) {
  var n = Number(d)
  if (!(n >= 1 && n <= OPTION_MAX)) return null
  return (options || [])[n - 1] || null
}

// ---------------------------------------------------------------- review queue state
// After a GET /review: more groups are waiting when the page was full and the queue is not loaded.
function reviewHasMore(loadedCount, remaining, lastPageLen) {
  return lastPageLen >= REVIEW_PAGE && loadedCount < remaining
}

// The offset of the next page. Groups that left the server's queue are gone from the loaded list,
// so the loaded groups are exactly the ones before the server's next unseen group.
function reviewNextOffset(rows) { return (rows || []).length }

// Progress of the queue as a fraction: how much of the starting queue is handled.
function reviewProgress(startRemaining, remaining) {
  var start = Number(startRemaining) || 0
  var left = Number(remaining) || 0
  if (start <= 0) return 1
  return Math.max(0, Math.min(1, (start - left) / start))
}

// ---------------------------------------------------------------- review queue: payee groups
// The queue has one entry per payee group (GET /review rows; each row describes the group's most
// recent row and carries group_ids). Per-group state is keyed by gkey(): the group key plus the sign.
//   unticked  {gkey: {id: true}}   every row is ticked unless listed here (ticked by default)
//   pending   {gkey: {category_id, path}}   the category y will apply; absent = the default
//   expanded  {gkey: true}          Enter shows the group's rows
//   rowsByKey {gkey: [rows]}        the expanded rows loaded so far (POST /smart/rows, 50 a page)
// Items are the flat list the view moves over: a group header, a loaded row, or "more".

// Group key: the normalized payee and the sign (out = money out, in = money in).
function gkey(g) { return String(g.group_key || "") + "|" + (Number(g.amount) < 0 ? "in" : "out") }

// The group's uncategorized row ids, as numbers (the representative's own id when the list is absent).
function groupIds(g) {
  if (!g) return []
  var ids = g.group_ids && g.group_ids.length ? g.group_ids : [g.id]
  var out = []
  for (var i = 0; i < ids.length; i++) {
    var v = Number(ids[i])
    if (isFinite(v)) out.push(v)
  }
  return out
}

// How many uncategorized rows the group has (the server's count; never less than the ids held).
function groupCount(g) {
  return Math.max(Number(g && g.group_count) || 0, groupIds(g).length)
}

function groupByKey(groups, key) {
  for (var i = 0; i < (groups || []).length; i++) if (gkey(groups[i]) === key) return groups[i]
  return null
}

function groupIndexOf(groups, key) {
  for (var i = 0; i < (groups || []).length; i++) if (gkey(groups[i]) === key) return i
  return -1
}

// The group without the given key (a new array).
function removeGroup(groups, key) {
  var out = []
  for (var i = 0; i < (groups || []).length; i++) if (gkey(groups[i]) !== key) out.push(groups[i])
  return out
}

// The ids the next "Enter: more" page asks /smart/rows for.
function groupPageIds(g, loaded) { return groupIds(g).slice(loaded, loaded + REVIEW_PAGE) }

function isTicked(unticked, key, id) {
  var u = unticked && unticked[key]
  return !(u && u[id])
}

function tickedIds(g, unticked) {
  var k = gkey(g)
  return groupIds(g).filter(function(id) { return isTicked(unticked, k, id) })
}

function tickedCount(g, unticked) { return tickedIds(g, unticked).length }

function allTicked(g, unticked) {
  var ids = groupIds(g)
  return ids.length > 0 && tickedCount(g, unticked) === ids.length
}

// A copy of unticked with the row id flipped (ticked <-> unticked). Empty entries are dropped.
function toggleTick(unticked, key, id) {
  var out = Object.assign({}, unticked)
  var u = Object.assign({}, out[key] || {})
  if (u[id]) delete u[id]
  else u[id] = true
  if (Object.keys(u).length === 0) delete out[key]
  else out[key] = u
  return out
}

// A copy of unticked with every row of the group ticked (on) or unticked (off).
function setGroupTicks(unticked, key, g, on) {
  var out = Object.assign({}, unticked)
  if (on) { delete out[key]; return out }
  var u = {}
  groupIds(g).forEach(function(id) { u[id] = true })
  out[key] = u
  return out
}

// The default pending category: the top option when suggestions are enabled, else none.
function defaultPending(g, enabled) {
  if (!g || enabled !== true) return null
  var top = (g.options || [])[0]
  if (!top) return null
  return { category_id: Number(top.category_id), path: String(top.path || "") }
}

// The pending category of a group: a chosen one, else the default.
function pendingFor(pending, g, enabled) {
  var p = pending && pending[gkey(g)]
  return p ? p : defaultPending(g, enabled)
}

// Pending as the header states it.
function pendingText(p) {
  if (!p) return "none: press 1/2/3 or c"
  return "category: " + String(p.path) + "   (y applies)"
}

// What y does for a group. ok false carries the toast text; no request is made then.
function applyPlan(g, pending, unticked) {
  if (!g) return { ok: false, message: "no group under the cursor" }
  if (!pending) return { ok: false, message: "no category yet: press 1, 2 or 3, or c to pick one" }
  var ids = tickedIds(g, unticked)
  if (ids.length === 0) return { ok: false, message: "no rows ticked: Space ticks a row, a ticks all" }
  return { ok: true, ids: ids, category_id: pending.category_id, path: String(pending.path) }
}

// \"Will categorize 38 of 41\".
function willText(g, unticked) {
  return "Will categorize " + fmtN(tickedCount(g, unticked)) + " of " + fmtN(groupCount(g))
}

// "2026-11-02" for one day, "2026-09-02 to 2026-11-18" for a span.
function dateSpan(g) {
  var a = String(g.group_date_min || g.date || "")
  var b = String(g.group_date_max || g.date || "")
  if (a === b || b === "") return a
  return a + " to " + b
}

// \"Netflix  ·  41 rows  ·  $1,234.56  ·  2026-09-02 to 2026-11-18\".
function groupLine(g) {
  var n = groupCount(g)
  return [String(g.payee || ""), fmtN(n) + (n === 1 ? " row" : " rows"), moneyCents(g.group_total),
          dateSpan(g)].join("  \u00b7  ")
}

// One transaction under an expanded group: "2026-11-02  AMAZON MKTP  $24.99  Chase Sapphire".
function rowLine(r) {
  return String(r.date || "") + "  " + String(r.payee || "") + "  " + moneyCents(r.amount) + "  " + String(r.account || "")
}

// Keeps the entries of map whose keys are in keys (a new object).
function pruneTo(map, keys) {
  var out = {}
  var src = map || {}
  for (var i = 0; i < (keys || []).length; i++) if (src[keys[i]] !== undefined) out[keys[i]] = src[keys[i]]
  return out
}

function groupKeys(groups) { return (groups || []).map(gkey) }

function anyExpanded(expanded) { return Object.keys(expanded || {}).length > 0 }

// The flat list the review view moves over. kinds: "group" (header), "row" (loaded row of an
// expanded group), "more" (rows of an expanded group still to load; Enter loads the next page).
function groupItems(groups, expanded, rowsByKey, loadingByKey) {
  var items = []
  for (var i = 0; i < (groups || []).length; i++) {
    var g = groups[i]
    var k = gkey(g)
    items.push({ kind: "group", key: "g:" + k, gkey: k, g: g })
    if (!expanded || !expanded[k]) continue
    var rows = (rowsByKey && rowsByKey[k]) || []
    for (var j = 0; j < rows.length; j++)
      items.push({ kind: "row", key: "r:" + k + ":" + Number(rows[j].id), gkey: k, g: g, row: rows[j] })
    var left = groupIds(g).length - rows.length
    if (left > 0)
      items.push({ kind: "more", key: "m:" + k, gkey: k, g: g, left: left, loaded: rows.length,
                   loading: !!(loadingByKey && loadingByKey[k]) })
  }
  return items
}

// The first group header at or after i, else before it; 0 when there are no items.
function nearestGroup(items, i) {
  var n = (items || []).length
  if (n === 0) return 0
  var start = clamp(i, n)
  for (var j = start; j < n; j++) if (items[j].kind === "group") return j
  for (var k = start; k >= 0; k--) if (items[k].kind === "group") return k
  return start
}

// Where the cursor is after the items changed: its key when still there, else the position it had
// (the next group takes it when a group left), moved to a group header.
function cursorIndex(items, key, fallback) {
  var i
  for (i = 0; i < (items || []).length; i++) if (items[i].key === key) return i
  // a row or "more" of a group that is still there: its header
  var gk = key.indexOf("r:") === 0 ? key.slice(2, key.lastIndexOf(":")) : key.indexOf("m:") === 0 ? key.slice(2) : null
  if (gk !== null)
    for (i = 0; i < (items || []).length; i++) if (items[i].key === "g:" + gk) return i
  return nearestGroup(items, fallback)
}

// s and Backspace: the next (dir +1) or previous (dir -1) group header; stays put when there is none.
function groupStep(items, i, dir) {
  for (var j = i + dir; j >= 0 && j < (items || []).length; j += dir) if (items[j].kind === "group") return j
  return i
}

// The review header line for the group under the cursor (index = its place in the queue).
function groupProgress(remaining, index, loaded) {
  var rem = Number(remaining) || 0
  if (rem <= 0) return "queue empty"
  return fmtN(rem) + (rem === 1 ? " group" : " groups") + " left  \u00b7  group " + (Number(index) + 1)
         + " of " + fmtN(Math.max(rem, Number(loaded) || 0))
}

// ---------------------------------------------------------------- dashboard tabs headers
function recurringHeader(list) {
  var active = 0
  for (var i = 0; i < (list || []).length; i++) if (list[i].active) active++
  var n = (list || []).length
  if (n === 0) return "No recurring series yet: a series needs 3 or more charges on a steady schedule."
  return active + " active of " + n + " series  \u00b7  " + money(activeAnnualTotal(list)) + "/yr from active series"
}

function anomaliesHeader(list) {
  var n = (list || []).length
  if (n === 0) return "No anomalies: no spikes, duplicates or new payees in the data."
  return n + (n === 1 ? " item" : " items") + ", newest first"
}

function transfersHeader(t) {
  var n = Number(t && t.count) || 0
  if (n === 0) return "No transfer pairs: opposite amounts between two accounts within 4 days."
  return n + (n === 1 ? " pair" : " pairs") + "  \u00b7  " + money(t.total) + " moved between accounts"
}

// Enter on a row of a tab: the query that jumps the table to it, or "" when there is nothing to jump to.
function jumpQueryFor(tab, row) {
  if (tab === "recurring") return payeeQuery(row.payee)
  if (tab === "anomalies") return idsQuery([row.txn_id])
  if (tab === "transfers") return idsQuery([row.out_id, row.in_id])
  return ""
}

// ---------------------------------------------------------------- header text (overlay title row)
function dashboardTitle(d) {
  if (!d) return "Dashboard"
  return "Dashboard  \u00b7  " + rangeText(d)
}
