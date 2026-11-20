import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "state"
import "state/Query.js" as Query
import "keys/Keymap.js" as Keymap
import "common"
import "views"
import "common/Fuzzy.js" as Fuzzy
import "state/Search.js" as Search

// OmaFinances window content: header, filter chips, sidebar | transaction table | detail, footer.
// Read-only. keys/Keymap.js is the source of truth for every key (docs/KEYS.md follows it).
// Overlays (picker, prompt, help) take the keyboard while open; this shell takes it otherwise.
FocusScope {
  id: root

  signal closeRequested()

  // ------------------------------------------------------------------ tuning
  readonly property int pageSize: 200
  // The next page is requested when the cursor gets this close to the loaded end.
  readonly property int prefetchRows: 40
  readonly property int sideWidth: Style.space(232)
  readonly property int detailWidth: Style.space(340)

  // ------------------------------------------------------------------ backend state
  // "loading" | "ok" | "unreachable" | "error" (an HTTP failure such as 401)
  property string backendState: "loading"
  property string backendDetail: ""
  property var serverStatus: null   // parsed /status body
  // Settings -> Appearance scale factor. shell.qml scales the whole UI by it.
  property real uiScale: 1.0

  readonly property bool dbError: serverStatus !== null && serverStatus.db !== undefined
                                  && serverStatus.db.state === "error"

  // ------------------------------------------------------------------ table state
  property int total: 0          // matched rows for the current filter (paging)
  property int allCount: 0       // all transactions in the database
  property real sumOut: 0
  property real sumIn: 0
  property bool summaryKnown: false
  property bool pageBusy: false
  property bool seekEnd: false   // G was pressed: keep loading until the last row is in
  property int rowsGen: 0        // bumped by a table reload; older responses are dropped
  property var restore: null     // {id, index}: keep the selected transaction across a reload
  property int cursor: 0         // highlighted table row

  // ------------------------------------------------------------------ query state
  // The one place table filters live. Query.buildQuery() is the only converter to a query string.
  property var filter: Query.emptyFilter()
  property var sort: ({ key: "date", dir: "desc" })

  // ------------------------------------------------------------------ panes
  // Keyboard focus: "sidebar" | "table" | "detail". Tab cycles the visible panes.
  property string zone: "table"
  property bool sidebarOpen: true
  property bool detailOpen: false
  property int sideIndex: 1

  // ------------------------------------------------------------------ detail
  property var detailData: null
  property string detailError: ""
  property bool detailLoading: false
  property int detailGen: 0

  // ------------------------------------------------------------------ facets (sidebar and pickers)
  property var accountList: []   // raw /accounts rows
  property var categoryList: []  // raw /categories rows
  property int uncategorizedCount: 0

  // Overlay context: which picker or prompt is up. Sub-pickers return to the filter menu.
  property string pickerCtx: ""  // "filter" | "account" | "category" | "date" | "flow" | "sort"
  property string promptCtx: ""  // "amount" | "text" | "date"
  property var presetList: []

  // ------------------------------------------------------------------ slash search and views
  property var viewList: []          // raw /views rows: {id, name, query, sort, position}
  property var historyRows: []       // GET /history, newest first
  property int histIndex: -1         // -1: the draft; otherwise the row of historyRows shown
  property string histDraft: ""
  property int parseGen: 0           // bumped on every bar edit; older /search/parse replies are dropped
  property var parsed: null          // Search.fromParse() of the text parsedText
  property string parsedText: ""
  property bool enterPending: false  // Enter was pressed before the reply for the text arrived
  property string cmdError: ""       // error of the last : command, shown in the hint row
  property var completionList: []    // [{insert, label, detail}] shown under the cursor
  property int completionIndex: 0
  property int completionStart: 0
  property int completionEnd: 0
  property string completionsFor: "" // the text and cursor the list was computed for
  property int completionsCursor: -1
  property string openedSlash: ""    // the active query when the bar opened (Esc restores it)
  property var openedSort: null
  property string pendingViewId: ""
  property string viewConfirmFrom: ""  // "views" | "sidebar": where the delete confirm came from
  readonly property bool searchOpen: searchBar.open
  readonly property bool commandMode: searchBar.text.charAt(0) === ":"

  // ------------------------------------------------------------------ models
  ListModel { id: rows }
  ListModel { id: sideRows }

  readonly property bool overlayOpen: picker.open || prompt.open || help.open || searchBar.open

  // ------------------------------------------------------------------ helpers
  function parseJson(text) {
    try { return JSON.parse(text) } catch (e) { return null }
  }

  function describeHttp(xhr) {
    if (xhr.status === 401) return "backend rejected the session token (401)"
    return "HTTP " + xhr.status
  }

  // The backend's {error} text when it sent one, else describeHttp().
  function httpError(xhr) {
    var j = parseJson(xhr.responseText)
    if (j && j.error) return String(j.error)
    return describeHttp(xhr)
  }

  function setUnreachable() {
    backendState = "unreachable"
    backendDetail = "port " + Backend.port
  }

  function toast(msg) { toastBar.show(msg) }

  function statusLine() {
    if (backendState === "loading" && serverStatus === null) return "connecting..."
    if (backendState === "unreachable") return "Backend not responding (" + backendDetail + ")"
    if (backendState === "error") return "Backend error: " + backendDetail
    if (serverStatus === null) return ""
    var s = serverStatus
    var db = s.db || {}
    var parts = ["v" + s.version,
                 Query.fmtCount(db.transactions || 0) + " txns",
                 Query.fmtCount(db.accounts || 0) + " accounts",
                 Query.fmtCount(db.categories || 0) + " categories"]
    if (db.last_txn_date) parts.push("last txn " + db.last_txn_date)
    if (db.last_sync) parts.push("synced " + db.last_sync)
    parts.push("mcp " + (s.mcp && s.mcp.enabled ? "on" : "off"))
    return parts.join("  \u00b7  ")
  }

  function summaryText() {
    if (!summaryKnown) return backendState === "ok" ? "" : "loading..."
    return Query.fmtCount(total) + " matched of " + Query.fmtCount(allCount)
         + "  \u00b7  out " + Query.fmtMoney(sumOut) + "  \u00b7  in " + Query.fmtMoney(sumIn)
  }

  function bodyMessage() {
    if (rows.count > 0) return ""
    if (backendState === "unreachable") return "Backend not responding"
    if (backendState === "error") return "Backend error: " + backendDetail
    if (!summaryKnown) return "Loading..."
    if (dbError) return "No transactions available: the database is locked."
    if (Query.isActive(filter)) return "No transactions match these filters.\nPress x to clear them."
    return "No transactions yet."
  }

  function currentTxid() {
    if (cursor < 0 || cursor >= rows.count) return -1
    return Number(rows.get(cursor).txid)
  }

  function indexOfTxid(id) {
    for (var i = 0; i < rows.count; i++) if (Number(rows.get(i).txid) === id) return i
    return -1
  }

  // ------------------------------------------------------------------ settings and status
  function loadSettings() {
    Backend.request("GET", "/settings", function(xhr) {
      if (xhr.status !== 200) return
      var j = parseJson(xhr.responseText)
      if (j && typeof j.scale_factor === "number" && j.scale_factor > 0)
        root.uiScale = Math.min(3, Math.max(0.5, j.scale_factor))
    })
  }

  function loadStatus() {
    Backend.request("GET", "/status", function(xhr) {
      if (xhr.status === 0) { setUnreachable(); return }
      var j = parseJson(xhr.responseText)
      if (xhr.status !== 200 || !j) {
        backendState = "error"
        backendDetail = describeHttp(xhr)
        return
      }
      serverStatus = j
      if (backendState === "unreachable" || backendState === "loading") backendState = "ok"
    })
  }

  // ------------------------------------------------------------------ facets: sidebar and pickers
  function loadFacets() {
    var pending = 2
    var accounts = [], categories = [], uncat = 0
    function finish() {
      pending--
      if (pending > 0) return
      accountList = accounts
      categoryList = categories
      uncategorizedCount = uncat
      rebuildSidebar()
    }
    Backend.request("GET", "/accounts", function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (j && j.rows) accounts = j.rows
      finish()
    })
    Backend.request("GET", "/categories", function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (j && j.rows) categories = j.rows
      if (j && j.uncategorized !== undefined && j.uncategorized !== null) {
        uncat = typeof j.uncategorized === "number" ? j.uncategorized : (Number(j.uncategorized.count) || 0)
      }
      finish()
    })
  }

  function isSideSkip(r) { return r.rowKind === "header" || r.rowKind === "note" }

  function rebuildSidebar() {
    sideRows.clear()
    sideRows.append({ rowKind: "header", key: "", label: "Views", depth: 0, count: 0, sel: false, query: "" })
    if (viewList.length === 0)
      sideRows.append({ rowKind: "note", key: "", label: "none yet: type / then :save <name>", depth: 0, count: 0, sel: false, query: "" })
    for (var v = 0; v < viewList.length; v++) {
      var vw = viewList[v]
      sideRows.append({ rowKind: "view", key: String(vw.id), label: String(vw.name), depth: 0, count: 0, sel: false,
                        query: String(vw.query || "") })
    }
    sideRows.append({ rowKind: "header", key: "", label: "Accounts", depth: 0, count: 0, sel: false, query: "" })
    for (var i = 0; i < accountList.length; i++) {
      var a = accountList[i]
      sideRows.append({ rowKind: "account", key: String(a.id), label: String(a.name || a.id),
                        depth: 0, count: Number(a.count) || 0, sel: false, query: "" })
    }
    sideRows.append({ rowKind: "header", key: "", label: "Categories", depth: 0, count: 0, sel: false, query: "" })
    for (var j = 0; j < categoryList.length; j++) {
      var c = categoryList[j]
      sideRows.append({ rowKind: "category", key: String(c.id), label: String(c.name || c.id),
                        depth: Number(c.depth) || 0, count: Number(c.count_subtree) || 0, sel: false, query: "" })
    }
    sideRows.append({ rowKind: "uncat", key: "none", label: "Uncategorized", depth: 0,
                      count: uncategorizedCount, sel: false, query: "" })
    refreshSideSelection()
    if (sideIndex < 1 || sideIndex >= sideRows.count) sideIndex = 1
    if (isSideSkip(sideRows.get(sideIndex))) moveSide(1)
  }

  // Marks the active account or category in the sidebar. Called after every filter change.
  function refreshSideSelection() {
    for (var i = 0; i < sideRows.count; i++) {
      var r = sideRows.get(i)
      var on = false
      if (r.rowKind === "account") on = filter.account !== "" && filter.account === r.key
      else if (r.rowKind === "category") on = filter.category !== "" && filter.category === r.key
      else if (r.rowKind === "uncat") on = filter.category === "none"
      else if (r.rowKind === "view") on = filter.slash !== "" && filter.slash === r.query
      if (r.sel !== on) sideRows.setProperty(i, "sel", on)
    }
  }

  function moveSide(dir) {
    var i = sideIndex + dir
    while (i >= 0 && i < sideRows.count && isSideSkip(sideRows.get(i))) i += dir
    if (i >= 0 && i < sideRows.count) sideIndex = i
  }

  // Enter applies the row's account or category (replacing that facet); Space toggles it.
  function sideActivate(toggle) {
    if (sideIndex < 0 || sideIndex >= sideRows.count) return
    var r = sideRows.get(sideIndex)
    if (isSideSkip(r)) return
    if (r.rowKind === "view") { applyViewId(r.key); return }
    var f = Object.assign({}, filter)
    if (r.rowKind === "account") {
      if (toggle && f.account === r.key) { f.account = ""; f.accountLabel = "" }
      else { f.account = r.key; f.accountLabel = r.label }
    } else {
      var cat = r.rowKind === "uncat" ? "none" : r.key
      if (toggle && f.category === cat) { f.category = ""; f.categoryLabel = "" }
      else { f.category = cat; f.categoryLabel = r.rowKind === "uncat" ? "" : r.label }
    }
    applyFilter(f)
  }

  // x in the sidebar: clear the facet the cursor is on.
  function clearSideFacet() {
    if (sideIndex < 0 || sideIndex >= sideRows.count) return
    var r = sideRows.get(sideIndex)
    var f = Object.assign({}, filter)
    if (r.rowKind === "account") { f.account = ""; f.accountLabel = "" }
    else if (r.rowKind === "category" || r.rowKind === "uncat") { f.category = ""; f.categoryLabel = "" }
    else return
    applyFilter(f)
  }

  // ------------------------------------------------------------------ table: loading and paging
  // Appends the next page. Offsets come from the rows already held, so a page is never requested
  // twice, and a reload cannot splice old rows into the new list.
  function loadPage() {
    if (pageBusy) return
    if (summaryKnown && rows.count >= total) {
      if (restore) settleSelection()
      else seekFinish()
      return
    }
    pageBusy = true
    var gen = rowsGen
    var url = Query.buildQuery(filter, sort, rows.count, pageSize)
    Backend.request("GET", url, function(xhr) {
      if (gen !== root.rowsGen) return
      pageBusy = false
      if (xhr.status === 0) { setUnreachable(); seekEnd = false; restore = null; return }
      var j = parseJson(xhr.responseText)
      if (xhr.status === 400) {
        // The UI validates input before it gets here, so this is a bug report, not a user error.
        toast("filter rejected: " + ((j && j.error) || "bad request"))
        seekEnd = false
        restore = null
        return
      }
      if (xhr.status !== 200 || !j || !j.rows) {
        backendState = "error"
        backendDetail = describeHttp(xhr)
        seekEnd = false
        restore = null
        return
      }
      for (var i = 0; i < j.rows.length; i++) {
        var r = j.rows[i]
        rows.append({
          txid: Number(r.id) || 0,
          date: String(r.date || "").substring(0, 10),
          payee: String(r.payee || ""),
          amount: Number(r.amount) || 0,
          currency: String(r.currency || ""),
          account: String(r.account || ""),
          category: r.category ? String(r.category) : "",
          pending: !!r.pending,
          recurring: !!r.recurring
        })
      }
      total = Number(j.total) || rows.count
      // An empty page with a larger total would otherwise keep asking forever.
      if (j.rows.length === 0) total = rows.count
      allCount = Number(j.all !== undefined ? j.all : j.total) || 0
      sumOut = Number(j.sum_out) || 0
      sumIn = Number(j.sum_in) || 0
      summaryKnown = true
      backendState = "ok"
      if (restore) settleSelection()
      else if (seekEnd) seekFinish()
      else maybePrefetch()
    })
  }

  // Rebuilds the table from the first page. The selected transaction is kept when it still
  // matches; otherwise the selection falls back to `fallbackIndex`.
  function reloadRows(fallbackIndex) {
    var id = currentTxid()
    rowsGen++
    rows.clear()
    total = 0
    summaryKnown = false
    pageBusy = false
    seekEnd = false
    restore = id >= 0 ? { id: id, index: fallbackIndex } : null
    if (!restore) cursor = 0
    loadPage()
  }

  function settleSelection() {
    if (!restore) return
    var i = indexOfTxid(restore.id)
    if (i >= 0) {
      restore = null
      select(i)
      return
    }
    if (rows.count >= total) {
      var fb = Math.max(0, Math.min(restore.index, rows.count - 1))
      restore = null
      select(fb)
      return
    }
    loadPage()
  }

  function maybePrefetch() {
    if (summaryKnown && rows.count < total && cursor >= rows.count - prefetchRows) loadPage()
  }

  // ------------------------------------------------------------------ table: navigation
  function select(i) {
    if (rows.count === 0) return
    cursor = Math.max(0, Math.min(rows.count - 1, i))
    maybePrefetch()
  }

  function moveRow(n) { select(cursor + n) }

  function pageStep() {
    return Math.max(1, Math.floor(table.listView.height / table.rowHeight / 2))
  }

  // G: jump to the real last row. Pages load until the total is in, then the selection moves.
  function goEnd() {
    if (rows.count === 0) return
    seekEnd = true
    if (rows.count < total) loadPage()
    else seekFinish()
  }

  function seekFinish() {
    if (!seekEnd) return
    if (rows.count < total) { loadPage(); return }
    seekEnd = false
    select(rows.count - 1)
  }

  // ------------------------------------------------------------------ filters and sort
  function applyFilter(f) {
    filter = f
    refreshSideSelection()
    reloadRows(0)
  }

  function clearAll() {
    if (!Query.isActive(filter)) { toast("no filters set"); return }
    applyFilter(Query.emptyFilter())
    toast("filters cleared")
  }

  // X: clear the newest chip (the last one in chip order).
  function clearNewestChip() {
    var list = Query.chips(filter)
    if (list.length === 0) { toast("no filters set"); return }
    applyFilter(Query.clearChip(filter, list[list.length - 1].id))
  }

  function removeChip(id) {
    applyFilter(Query.clearChip(filter, id))
  }

  function applySort(key) {
    sort = Query.nextSort(sort, key)
    reloadRows(0)
  }

  function openFilterMenu() {
    var f = filter
    pickerCtx = "filter"
    picker.title = "Filter"
    picker.hint = ""
    picker.emptyText = "no matching filter"
    picker.currentKey = ""
    picker.entries = [
      { key: "account", label: "Account", detail: f.account ? (f.accountLabel || f.account) : "any" },
      { key: "category", label: "Category", detail: f.category === "none" ? "uncategorized" : (f.category ? (f.categoryLabel || f.category) : "any") },
      { key: "date", label: "Date range", detail: f.from ? (f.from === f.to ? f.from : f.from + ".." + f.to) : "any" },
      { key: "amount", label: "Amount", detail: f.amountLabel || "any" },
      { key: "flow", label: "Money out / in / all", detail: f.flow === "out" ? "money out" : (f.flow === "in" ? "money in" : "all") },
      { key: "text", label: "Text contains", detail: f.text ? "\"" + f.text + "\"" : "any" },
      { key: "clear", label: "Clear all filters", detail: Query.isActive(f) ? "" : "none set" }
    ]
    picker.show()
  }

  function openAccountPicker() {
    pickerCtx = "account"
    picker.title = "Account"
    picker.hint = "Matches transactions in that account."
    picker.emptyText = "no matching account"
    var list = [{ key: "", label: "All accounts", detail: filter.account === "" ? "current" : "" }]
    for (var i = 0; i < accountList.length; i++) {
      var a = accountList[i]
      list.push({ key: String(a.id), label: String(a.name || a.id),
                  detail: (a.institution ? a.institution + "  \u00b7  " : "") + Query.fmtCount(a.count) + " txns" })
    }
    picker.entries = list
    picker.currentKey = ""   // always start on "All accounts", like OmaLook
    picker.show()
  }

  function openCategoryPicker() {
    pickerCtx = "category"
    picker.title = "Category"
    picker.hint = "A category includes its subcategories."
    picker.emptyText = "no matching category"
    var list = [{ key: "", label: "All categories", detail: filter.category === "" ? "current" : "" },
                { key: "none", label: "Uncategorized", detail: Query.fmtCount(uncategorizedCount) + " txns" }]
    for (var i = 0; i < categoryList.length; i++) {
      var c = categoryList[i]
      var indent = ""
      for (var d = 0; d < (Number(c.depth) || 0); d++) indent += "   "
      list.push({ key: String(c.id), label: indent + String(c.name || c.id),
                  detail: Query.fmtCount(c.count_subtree) + " txns" })
    }
    picker.entries = list
    picker.currentKey = ""   // always start on "All categories"
    picker.show()
  }

  function openDatePicker() {
    pickerCtx = "date"
    picker.title = "Date range"
    picker.hint = "Presets are relative to today. Custom: type YYYY-MM-DD..YYYY-MM-DD."
    picker.emptyText = "no matching range"
    presetList = Query.presets(new Date())
    var list = [{ key: "any", label: "Any date", detail: "" }]
    for (var i = 0; i < presetList.length; i++) {
      var p = presetList[i]
      list.push({ key: p.key, label: p.label, detail: p.from + ".." + p.to })
    }
    list.push({ key: "custom", label: "Custom range...", detail: "type it" })
    picker.entries = list
    picker.currentKey = ""
    picker.show()
  }

  function openFlowPicker() {
    pickerCtx = "flow"
    picker.title = "Money out / in / all"
    picker.hint = "Money out is spending (positive amounts); money in is income and refunds (negative)."
    picker.emptyText = "no matching choice"
    picker.entries = [
      { key: "all", label: "All money", detail: "" },
      { key: "out", label: "Money out", detail: "spending" },
      { key: "in", label: "Money in", detail: "income, refunds" }
    ]
    picker.currentKey = filter.flow === "" ? "all" : filter.flow
    picker.show()
  }

  function openSortPicker() {
    pickerCtx = "sort"
    picker.title = "Sort"
    picker.hint = "Choosing the current column reverses the order."
    picker.emptyText = "no matching column"
    var list = []
    for (var i = 0; i < Query.SORT_KEYS.length; i++) {
      var s = Query.SORT_KEYS[i]
      list.push({ key: s.key, label: s.label,
                  detail: s.key === sort.key ? Query.arrow(sort) + " current" : "" })
    }
    picker.entries = list
    picker.currentKey = sort.key
    picker.show()
  }

  function openPrompt(ctx) {
    promptCtx = ctx
    if (ctx === "amount") {
      prompt.title = "Amount"
      prompt.hint = "10..50 between two amounts, >100 above, <20 below. Money out is positive, money in negative."
      prompt.placeholder = "e.g. 10..50"
      prompt.validator = function(t) { var r = Query.parseAmount(t); return r.error || "" }
      prompt.show(filter.amountLabel)
    } else if (ctx === "text") {
      prompt.title = "Text contains"
      prompt.hint = "Matches payee and other text fields. Empty clears it."
      prompt.placeholder = "e.g. coffee"
      prompt.validator = function(t) { var r = Query.parseText(t); return r.error || "" }
      prompt.show(filter.text)
    } else {
      prompt.title = "Custom date range"
      prompt.hint = "YYYY-MM-DD..YYYY-MM-DD, or one day as YYYY-MM-DD. Both ends are included."
      prompt.placeholder = "2026-11-01..2026-11-20"
      prompt.validator = function(t) { var r = Query.parseDateRange(t); return r.error || "" }
      prompt.show(filter.from ? (filter.from === filter.to ? filter.from : filter.from + ".." + filter.to) : "")
    }
  }

  // The prompt has already validated the text, so the parsers below only ever see good input.
  function onPromptSubmitted(text) {
    var f = Object.assign({}, filter)
    if (promptCtx === "amount") {
      var a = Query.parseAmount(text)
      f.min = a.clear ? "" : a.min
      f.max = a.clear ? "" : a.max
      f.amountLabel = a.clear ? "" : a.label
    } else if (promptCtx === "text") {
      var t = Query.parseText(text)
      f.text = t.clear ? "" : t.text
    } else {
      var d = Query.parseDateRange(text)
      f.from = d.clear ? "" : d.from
      f.to = d.clear ? "" : d.to
    }
    applyFilter(f)
  }

  function onPickerChosen(key) {
    var ctx = pickerCtx
    var f
    if (ctx === "views") { applyViewId(key); return }
    if (ctx === "viewConfirm") {
      var from = viewConfirmFrom
      var dv = viewById(pendingViewId)
      if (key === "yes" && dv) deleteView(dv, function() { if (from === "views") openViewsPicker() })
      else if (from === "views") openViewsPicker()
      return
    }
    if (ctx === "filter") {
      if (key === "account") openAccountPicker()
      else if (key === "category") openCategoryPicker()
      else if (key === "date") openDatePicker()
      else if (key === "amount") openPrompt("amount")
      else if (key === "flow") openFlowPicker()
      else if (key === "text") openPrompt("text")
      else if (key === "clear") clearAll()
    } else if (ctx === "account") {
      f = Object.assign({}, filter)
      f.account = key
      f.accountLabel = ""
      if (key !== "") {
        var acc = accountList.filter(function(a) { return String(a.id) === key })[0]
        f.accountLabel = acc ? String(acc.name || key) : key
      }
      applyFilter(f)
    } else if (ctx === "category") {
      f = Object.assign({}, filter)
      f.category = key
      f.categoryLabel = ""
      if (key !== "" && key !== "none") {
        var cat = categoryList.filter(function(c) { return String(c.id) === key })[0]
        f.categoryLabel = cat ? String(cat.name || key) : key
      }
      applyFilter(f)
    } else if (ctx === "date") {
      if (key === "custom") { openPrompt("date"); return }
      f = Object.assign({}, filter)
      if (key === "any") { f.from = ""; f.to = "" }
      else {
        var p = presetList.filter(function(x) { return x.key === key })[0]
        if (p) { f.from = p.from; f.to = p.to }
      }
      applyFilter(f)
    } else if (ctx === "flow") {
      f = Object.assign({}, filter)
      f.flow = key === "all" ? "" : key
      applyFilter(f)
    } else if (ctx === "sort") {
      applySort(key)
    }
  }

  function onPickerCancelled() {
    if (pickerCtx === "views") return
    if (pickerCtx === "viewConfirm") { if (viewConfirmFrom === "views") openViewsPicker(); return }
    // A sub-picker opened from the filter menu goes back to that menu.
    if (pickerCtx !== "filter" && pickerCtx !== "sort") openFilterMenu()
  }

  function onPromptCancelled() {
    if (promptCtx === "date") openDatePicker()
    else openFilterMenu()
  }

  // Give keys back to the shell after an overlay closes. Deferred: the closing overlay hides its
  // text field after onOpenChanged runs, which would otherwise take focus away again. Skipped if
  // another overlay reopened in the meantime (a sub-picker, for example).
  function restoreFocus() {
    if (overlayOpen) return
    // An overlay's text field keeps the window's active focus after the overlay is hidden, so keys
    // would go to an invisible field. Drop it explicitly, then take focus back.
    var a = root.Window.activeFocusItem
    if (a && a !== root) a.focus = false
    picker.focus = false; prompt.focus = false; help.focus = false
    searchBar.field.focus = false
    root.forceActiveFocus()
  }

  // ------------------------------------------------------------------ detail
  function loadDetail() {
    var id = currentTxid()
    var g = ++detailGen
    if (id < 0) {
      detailData = null
      detailError = ""
      detailLoading = false
      return
    }
    detailLoading = true
    Backend.request("GET", "/transactions/" + id, function(xhr) {
      if (g !== root.detailGen) return
      detailLoading = false
      if (xhr.status === 200) {
        var j = parseJson(xhr.responseText)
        if (j) { detailData = j; detailError = ""; return }
      }
      detailData = null
      detailError = xhr.status === 0 ? "backend not responding"
                                     : "could not load this transaction (" + describeHttp(xhr) + ")"
    })
  }

  function toggleDetail() {
    detailOpen = !detailOpen
    if (!detailOpen && zone === "detail") zone = "table"
    if (detailOpen) { detailData = null; loadDetail() }
  }

  // ------------------------------------------------------------------ reload and refresh
  function reload() {
    backendState = "loading"
    backendDetail = ""
    serverStatus = null
    loadStatus()
    loadFacets()
    loadViews()
    reloadRows(cursor)
  }

  // r: ask the backend to pull from Lunch Money, then reload everything.
  function refresh() {
    toast("refreshing from Lunch Money...")
    Backend.request("POST", "/refresh", function(xhr) {
      if (xhr.status === 0) { setUnreachable(); toast("backend not responding"); return }
      if (xhr.status !== 200) { toast("refresh failed: " + describeHttp(xhr)); return }
      var j = parseJson(xhr.responseText)
      toast("refreshed: " + Query.fmtCount(j ? j.transactions : 0) + " transactions")
      reload()
    })
  }

  // ------------------------------------------------------------------ slash search
  // The bar is a view of this state. Replies from /search/parse apply the query live (the last
  // good one stays when the text does not parse); Enter pins it; Esc restores the query from
  // when the bar opened.

  readonly property string searchHelpHint: "words match payee or notes  ·  p: payee  a: account  c: category  d: date  is:  sort:  ·  -term excludes  ·  :help"
  readonly property string commandsHint: "commands: :save <name>  :view <name>  :views  :delete <name>  :clear  :help"

  // Applies a canonical query and, when given, a sort as one change of the table.
  function applySlash(canonical, sortObj) {
    var sortChanged = sortObj !== null && sortObj !== undefined
                      && (sortObj.key !== sort.key || sortObj.dir !== sort.dir)
    if (sortChanged) sort = { key: sortObj.key, dir: sortObj.dir }
    if (canonical === filter.slash && !sortChanged) return
    var f = Object.assign({}, filter)
    f.slash = canonical
    applyFilter(f)
  }

  function openSearch() {
    if (searchBar.open) return
    openedSlash = filter.slash
    openedSort = { key: sort.key, dir: sort.dir }
    histIndex = -1
    histDraft = ""
    parsed = null
    parsedText = ""
    cmdError = ""
    enterPending = false
    loadHistory()
    searchBar.show(filter.slash)
    refreshBar()
  }

  // Esc (restore = true) puts back the query and sort from when the bar opened; Enter and the
  // commands close without restoring.
  function closeSearch(restore) {
    if (!searchBar.open) return
    parseTimer.stop()
    if (restore) applySlash(openedSlash, openedSort)
    searchBar.hide()
    zone = "table"
  }

  function onUserEdit() {
    histIndex = -1
    cmdError = ""
  }

  function scheduleParse() {
    parseGen++
    if (!searchBar.open) return
    if (commandMode) {
      parseTimer.stop()
      refreshCommandCompletions()
      refreshBar()
      return
    }
    parseTimer.restart()
    refreshBar()
  }

  function setCompletions(text, cur, items, start, end) {
    completionsFor = text
    completionsCursor = cur
    completionList = items
    completionStart = start
    completionEnd = end
    if (completionIndex >= items.length) completionIndex = 0
  }

  // A list is only acted on when it was computed for the current text and cursor.
  function completionFresh() {
    return completionsFor === searchBar.text && completionsCursor === searchBar.cursor
  }

  function refreshCommandCompletions() {
    var t = searchBar.text
    var cur = searchBar.cursor
    var cc = Search.commandCompletions(t, cur, viewNames())
    setCompletions(t, cur, cc ? cc.items : [], cc ? cc.start : 0, cc ? cc.end : 0)
  }

  function sendParse() {
    if (!searchBar.open || commandMode) return
    var text = searchBar.text
    var cur = searchBar.cursor
    var gen = parseGen
    if (text.trim() === "") {
      parsed = { ok: true, canonical: "", matched: null, sortValue: null, error: null }
      parsedText = text
      setCompletions(text, cur, [], 0, 0)
      applySlash("", null)
      refreshBar()
      return
    }
    var pos = Search.byteOffset(text, cur)
    Backend.request("GET", "/search/parse?q=" + encodeURIComponent(text) + "&pos=" + pos, function(xhr) {
      if (gen !== root.parseGen || !searchBar.open) return
      if (xhr.status === 0) { root.hintFail("search: backend not responding"); return }
      var j = root.parseJson(xhr.responseText)
      if (xhr.status !== 200 || !j) { root.hintFail("search: " + root.httpError(xhr)); return }
      var p = Search.fromParse(text, j)
      root.parsed = p
      root.parsedText = text
      root.setCompletions(text, cur, p.completions.items, p.completions.start, p.completions.end)
      if (p.ok) {
        var sv = p.sortValue !== null ? Search.sortFromValue(p.sortValue) : null
        root.applySlash(p.canonical, sv)
      }
      root.refreshBar()
      if (root.enterPending) {
        root.enterPending = false
        if (searchBar.text === text) root.submitSearch()
      }
    })
  }

  function hintFail(msg) {
    searchBar.hint = msg
    searchBar.hintError = true
    searchBar.errorStart = -1
    searchBar.errorEnd = -1
  }

  // Everything the bar shows is derived here from the state above.
  function refreshBar() {
    var t = searchBar.text
    searchBar.completions = completionList
    searchBar.completionIndex = completionIndex
    searchBar.completionStart = completionStart
    if (commandMode) {
      searchBar.errorStart = -1
      searchBar.errorEnd = -1
      var pc = Search.parseCommand(t)
      var showErr = cmdError !== "" || (pc !== null && pc.error && /\s/.test(t))
      if (showErr) {
        searchBar.hint = cmdError !== "" ? cmdError : pc.error
        searchBar.hintError = true
      } else if (pc !== null && pc.cmd) {
        searchBar.hint = ":" + pc.cmd + "  " + Search.COMMAND_HELP[pc.cmd]
        searchBar.hintError = false
      } else {
        searchBar.hint = commandsHint
        searchBar.hintError = false
      }
      return
    }
    if (t.trim() === "") {
      searchBar.hint = searchHelpHint
      searchBar.hintError = false
      searchBar.errorStart = -1
      searchBar.errorEnd = -1
      return
    }
    if (parsed === null) {
      searchBar.hint = "searching..."
      searchBar.hintError = false
      return
    }
    if (parsed.error) {
      searchBar.hint = parsed.error.message
      searchBar.hintError = true
      searchBar.errorStart = parsed.error.start
      searchBar.errorEnd = parsed.error.end
      return
    }
    searchBar.errorStart = -1
    searchBar.errorEnd = -1
    searchBar.hintError = false
    if (parsed.canonical === "") { searchBar.hint = searchHelpHint; return }
    var m = parsed.matched !== null ? Query.fmtCount(parsed.matched) + " matched  ·  " : ""
    searchBar.hint = m + parsed.canonical
  }

  function moveCompletion(d) {
    var n = completionList.length
    if (n === 0) return
    completionIndex = (completionIndex + d + n) % n
    searchBar.completionIndex = completionIndex
  }

  // Replaces the token under the cursor with the completion and adds a space after it.
  function acceptCompletion(i) {
    if (!completionFresh() || i < 0 || i >= completionList.length) return
    var r = Search.replaceRange(searchBar.text, completionStart, completionEnd, completionList[i].insert)
    completionList = []
    searchBar.text = r.text
    searchBar.cursor = r.cursor
  }

  function walkHistory(dir) {
    var r = Search.walkHistory(histIndex, histDraft, historyRows, searchBar.text, dir)
    if (r === null) return
    histIndex = r.index
    histDraft = r.draft
    searchBar.text = r.text
    searchBar.cursor = r.text.length
  }

  function onSearchAction(id) {
    if (id === "search.cancel") closeSearch(true)
    else if (id === "search.submit") submitSearch()
    else if (id === "search.complete") { if (completionList.length > 0) acceptCompletion(completionIndex) }
    else if (id === "search.backTab") moveCompletion(-1)
    else if (id === "search.up") { if (completionList.length > 0) moveCompletion(-1); else walkHistory(-1) }
    else if (id === "search.down") { if (completionList.length > 0) moveCompletion(1); else walkHistory(1) }
    else if (id === "search.clear") { searchBar.text = ""; searchBar.cursor = 0 }
    else if (id === "search.delWord") {
      var r = Search.deleteWordBack(searchBar.text, searchBar.cursor)
      searchBar.text = r.text
      searchBar.cursor = r.cursor
    }
  }

  function submitSearch() {
    var t = searchBar.text
    if (commandMode) { runCommand(t); return }
    if (t.trim() === "") { closeSearch(false); applySlash("", null); return }
    if (parsedText !== t || parsed === null) {
      enterPending = true
      parseTimer.stop()
      sendParse()
      return
    }
    if (parsed.error) { refreshBar(); return }
    applySlash(parsed.canonical, parsed.sortValue !== null ? Search.sortFromValue(parsed.sortValue) : null)
    Backend.json("POST", "/history", { q: t }, function(xhr) {
      if (xhr.status === 200) loadHistory()
    })
    closeSearch(false)
  }

  function runCommand(t) {
    var pc = Search.parseCommand(t)
    if (pc === null) return
    if (pc.error) { cmdError = pc.error; refreshBar(); return }
    if (pc.cmd === "save") saveView(pc.arg)
    else if (pc.cmd === "view") applyViewByName(pc.arg)
    else if (pc.cmd === "views") { closeSearch(false); openViewsPicker() }
    else if (pc.cmd === "delete") deleteViewByName(pc.arg)
    else if (pc.cmd === "clear") { closeSearch(false); clearAll() }
    else if (pc.cmd === "help") { closeSearch(false); help.focusedZone = zone; help.show() }
  }

  // The backend's {error} text when it sent one, else the status.
  function loadHistory() {
    Backend.request("GET", "/history", function(xhr) {
      if (xhr.status !== 200) return
      var j = parseJson(xhr.responseText)
      if (j && j.rows) historyRows = j.rows
    })
  }

  // ------------------------------------------------------------------ saved views
  function viewById(id) {
    for (var i = 0; i < viewList.length; i++) if (String(viewList[i].id) === String(id)) return viewList[i]
    return null
  }

  function viewNames() {
    var out = []
    for (var i = 0; i < viewList.length; i++) out.push(String(viewList[i].name))
    return out
  }

  function sortLabelOf(v) {
    var s = v.sort ? Search.sortFromValue(v.sort) : null
    if (!s) return ""
    return Query.sortInfo(s.key).label.toLowerCase() + " " + Query.arrow(s)
  }

  function loadViews(done) {
    Backend.request("GET", "/views", function(xhr) {
      if (xhr.status === 200) {
        var j = parseJson(xhr.responseText)
        if (j && j.rows) { viewList = j.rows; rebuildSidebar() }
      }
      if (done) done(xhr.status)
    })
  }

  // Saves the active query and sort under name. Errors stay in the bar.
  function saveView(name) {
    if (filter.slash === "") {
      cmdError = "nothing to save: no query is active; type a search first"
      refreshBar()
      return
    }
    Backend.json("POST", "/views", { name: name, query: filter.slash, sort: Search.sortToValue(sort) }, function(xhr) {
      if (xhr.status === 0) { cmdError = "backend not responding"; refreshBar(); return }
      var j = parseJson(xhr.responseText)
      if (xhr.status !== 200 || !j) { cmdError = httpError(xhr); refreshBar(); return }
      closeSearch(false)
      toast("saved view '" + name + "'")
      loadViews()
    })
  }

  function applyViewId(id) {
    var v = viewById(id)
    if (!v) return
    applySlash(String(v.query || ""), v.sort ? Search.sortFromValue(v.sort) : null)
    toast("view: " + v.name)
  }

  function applyViewByName(name) {
    var entries = []
    for (var i = 0; i < viewList.length; i++)
      entries.push({ key: String(viewList[i].id), label: String(viewList[i].name), detail: "" })
    var hits = Fuzzy.filterEntries(entries, name)
    if (hits.length === 0) { cmdError = "no view matches '" + name + "'"; refreshBar(); return }
    var id = hits[0].key
    closeSearch(false)
    applyViewId(id)
  }

  function deleteView(v, done) {
    Backend.request("DELETE", "/views/" + v.id, function(xhr) {
      var err = xhr.status === 0 ? "backend not responding" : (xhr.status !== 200 ? httpError(xhr) : "")
      toast(err !== "" ? "delete failed: " + err : "deleted view '" + v.name + "'")
      loadViews(function() { if (done) done(err) })
    })
  }

  function deleteViewByName(name) {
    var want = name.toLowerCase()
    var hit = null
    for (var i = 0; i < viewList.length; i++)
      if (String(viewList[i].name).toLowerCase() === want) hit = viewList[i]
    if (hit === null) { cmdError = "no view named '" + name + "'"; refreshBar(); return }
    deleteView(hit, function(err) {
      if (err !== "") { cmdError = err; refreshBar() }
      else closeSearch(false)
    })
  }

  function openViewsPicker() {
    pickerCtx = "views"
    picker.title = "Saved views"
    picker.hint = viewList.length > 0
        ? "Enter applies a view: it replaces the search and sets its sort. d deletes."
        : "No saved views yet. Type / then :save <name> to save the active query."
    picker.emptyText = "no matching view"
    picker.deletable = true
    var list = []
    for (var i = 0; i < viewList.length; i++) {
      var v = viewList[i]
      var sl = sortLabelOf(v)
      list.push({ key: String(v.id), label: String(v.name), detail: String(v.query || "") + (sl !== "" ? "  ·  " + sl : "") })
    }
    picker.entries = list
    picker.currentKey = ""
    picker.show()
  }

  function openViewDeleteConfirm(key, from) {
    var v = viewById(key)
    if (!v) return
    pendingViewId = String(v.id)
    viewConfirmFrom = from
    pickerCtx = "viewConfirm"
    picker.title = "Delete view '" + v.name + "'?"
    picker.hint = "Removes the saved query. Transactions are not changed."
    picker.emptyText = ""
    picker.deletable = false
    picker.entries = [ { key: "yes", label: "Delete it", detail: "" }, { key: "no", label: "Keep it", detail: "" } ]
    picker.currentKey = ""
    picker.show()
  }

  function deleteCursorView() {
    if (sideIndex < 0 || sideIndex >= sideRows.count) return
    var r = sideRows.get(sideIndex)
    if (r.rowKind === "view") openViewDeleteConfirm(r.key, "sidebar")
  }

  // ------------------------------------------------------------------ focus and keys
  function zones() {
    var z = []
    if (sidebarOpen) z.push("sidebar")
    z.push("table")
    if (detailOpen) z.push("detail")
    return z
  }

  // Left/Right arrows: move between neighbouring panes without wrapping. Left from the table
  // opens a hidden sidebar so there is always something to the left.
  function paneStep(dir) {
    var z = zones()
    var i = z.indexOf(zone)
    if (dir < 0 && zone === "table" && !sidebarOpen) { sidebarOpen = true; zone = "sidebar"; return }
    var n = i + dir
    if (n >= 0 && n < z.length) zone = z[n]
  }

  function cycleFocus(dir) {
    var z = zones()
    var i = z.indexOf(zone)
    zone = z[(Math.max(0, i) + dir + z.length) % z.length]
  }

  // The focused pane's scope is checked before "global", so a pane can shadow a global key.
  function scopesNow() {
    if (zone === "sidebar") return ["sidebar", "global"]
    if (zone === "detail") return ["detail", "global"]
    return ["table", "global"]
  }

  function run(id) {
    switch (id) {
      case "help": help.focusedZone = zone; help.show(); break
      case "filter.menu": openFilterMenu(); break
      case "sort.menu": openSortPicker(); break
      case "sort.date": applySort("date"); break
      case "sort.amount": applySort("amount"); break
      case "sort.payee": applySort("payee"); break
      case "sort.account": applySort("account"); break
      case "sort.category": applySort("category"); break
      case "sidebar.toggle":
        sidebarOpen = !sidebarOpen
        if (!sidebarOpen && zone === "sidebar") zone = "table"
        break
      case "focus.next": cycleFocus(1); break
      case "focus.prev": cycleFocus(-1); break
      case "pane.left": paneStep(-1); break
      case "pane.right": paneStep(1); break
      case "focus.table": zone = "table"; break
      case "detail.toggle": toggleDetail(); break
      case "detail.close": detailOpen = false; zone = "table"; break
      case "detail.down": detailPane.scrollBy(40); break
      case "detail.up": detailPane.scrollBy(-40); break
      case "detail.pagedown": detailPane.scrollBy(detailPane.height * 0.9); break
      case "detail.pageup": detailPane.scrollBy(-detailPane.height * 0.9); break
      case "side.pagedown": for (var pd = 0; pd < 10; pd++) moveSide(1); break
      case "side.pageup": for (var pu = 0; pu < 10; pu++) moveSide(-1); break
      case "filter.clearAll": clearAll(); break
      case "filter.clearLast": clearNewestChip(); break
      case "filter.clearFacet": clearSideFacet(); break
      case "reload": refresh(); break
      case "search.open": openSearch(); break
      case "views.open": openViewsPicker(); break
      case "view.delete": deleteCursorView(); break
      case "quit": root.closeRequested(); break
      case "move.down": if (zone === "sidebar") moveSide(1); else moveRow(1); break
      case "move.up": if (zone === "sidebar") moveSide(-1); else moveRow(-1); break
      case "move.first": select(0); break
      case "move.last": goEnd(); break
      case "page.down": moveRow(pageStep()); break
      case "page.up": moveRow(-pageStep()); break
      case "page.fulldown": moveRow(pageStep() * 2); break
      case "page.fullup": moveRow(-pageStep() * 2); break
      case "side.apply": sideActivate(false); break
      case "side.toggle": sideActivate(true); break
    }
  }

  Component.onCompleted: {
    root.forceActiveFocus()
    loadSettings()
    reload()
  }

  // Status line refresh (counts, db state, last sync) without touching the loaded rows.
  Timer {
    interval: 15000
    running: true
    repeat: true
    onTriggered: root.loadStatus()
  }

  // The bar's reply is requested after the typing pauses for this long.
  Timer {
    id: parseTimer
    interval: 80
    onTriggered: root.sendParse()
  }

  // Detail follows the cursor, debounced so holding j does not send a request per row.
  Timer {
    id: detailTimer
    interval: 120
    onTriggered: if (root.detailOpen) root.loadDetail()
  }

  onCursorChanged: {
    if (detailOpen) {
      detailData = null
      detailTimer.restart()
    }
  }

  onSidebarOpenChanged: if (!sidebarOpen && zone === "sidebar") zone = "table"

  // Keys: only when no overlay is up. The overlays take their own keys while open.
  focus: true
  Keys.onPressed: function(e) {
    if (overlayOpen) return
    var id = Keymap.resolve(scopesNow(), Keymap.token(e))
    if (id === "") return
    e.accepted = true
    run(id)
  }

  // ------------------------------------------------------------------ layout
  ColumnLayout {
    anchors.fill: parent
    anchors.margins: Style.spacing.xxl
    spacing: 0

    // Header row 1: app name and the backend status line.
    RowLayout {
      Layout.fillWidth: true
      spacing: Style.spacing.lg * 2

      Text {
        text: "OmaFinances"
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.heading
        font.bold: true
      }
      Text {
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignVCenter
        text: root.statusLine()
        color: root.backendState === "unreachable" || root.backendState === "error"
               ? Commons.Color.urgent : Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        elide: Text.ElideRight
        horizontalAlignment: Text.AlignRight
      }
    }

    // Database error banner: the message, then the fix.
    Rectangle {
      visible: root.dbError
      Layout.fillWidth: true
      Layout.topMargin: Style.spacing.lg
      implicitHeight: bannerCol.implicitHeight + Style.spacing.lg * 2
      color: Qt.alpha(Commons.Color.urgent, 0.12)
      border.width: Style.space(1)
      border.color: Commons.Color.urgent
      radius: Style.cornerRadius

      ColumnLayout {
        id: bannerCol
        anchors.fill: parent
        anchors.margins: Style.spacing.lg
        spacing: Style.spacing.xs
        Text {
          Layout.fillWidth: true
          text: root.serverStatus && root.serverStatus.db ? (root.serverStatus.db.message || "Database error") : ""
          color: Commons.Color.urgent
          font.family: Style.font.family
          font.pixelSize: Style.font.subtitle
          font.bold: true
          wrapMode: Text.WordWrap
        }
        Text {
          Layout.fillWidth: true
          text: "run: omafinances keyring-import"
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }
    }

    // Header row 2: the match summary, and the sort with its arrow.
    RowLayout {
      Layout.fillWidth: true
      Layout.topMargin: Style.spacing.lg
      spacing: Style.spacing.lg * 2
      Text {
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        text: root.summaryText()
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.body
        elide: Text.ElideRight
      }
      Text {
        text: "sort: " + Query.sortInfo(root.sort.key).label.toLowerCase() + " " + Query.arrow(root.sort)
        color: Commons.Color.accent
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    // Header row 3: active filters as chips (click one to clear it).
    FilterChips {
      id: chipRow
      Layout.fillWidth: true
      Layout.topMargin: Style.spacing.md
      chips: Query.chips(root.filter)
      onChipClicked: function(id) { root.removeChip(id) }
    }

    // Body: sidebar | table | detail.
    RowLayout {
      id: body
      Layout.fillWidth: true
      Layout.fillHeight: true
      Layout.topMargin: Style.spacing.lg
      spacing: Style.spacing.lg

      Sidebar {
        id: sidebar
        visible: root.sidebarOpen
        Layout.preferredWidth: root.sidebarOpen ? root.sideWidth : 0
        Layout.fillHeight: true
        rowsModel: sideRows
        cursor: root.sideIndex
        focusedPane: root.zone === "sidebar"
        onRowClicked: function(i) {
          root.zone = "sidebar"
          root.sideIndex = i
          root.sideActivate(true)
        }
      }

      Item {
        id: tableArea
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumWidth: 0

        SearchBar {
          id: searchBar
          anchors.top: parent.top
          anchors.left: parent.left
          anchors.right: parent.right
          onAction: function(id) { root.onSearchAction(id) }
          onUserEdited: root.onUserEdit()
          onCompletionPicked: function(i) { root.acceptCompletion(i) }
          onCursorChanged: root.scheduleParse()
          onTextChanged: root.scheduleParse()
          onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
        }

        TxTable {
          id: table
          anchors.top: searchBar.bottom
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.bottom: parent.bottom
          rowsModel: rows
          currentIndex: root.cursor
          focusedPane: root.zone === "table"
          sortKey: root.sort.key
          sortDir: root.sort.dir
          prefetchRows: root.prefetchRows
          onRowClicked: function(i) { root.zone = "table"; root.select(i) }
          onColumnClicked: function(key) { root.applySort(key) }
          onNearEnd: root.loadPage()
        }

        // Empty states, over the table area.
        Text {
          anchors.centerIn: parent
          visible: rows.count === 0 && root.bodyMessage() !== ""
          width: parent.width - Style.space(40)
          horizontalAlignment: Text.AlignHCenter
          wrapMode: Text.WordWrap
          text: root.bodyMessage()
          color: root.backendState === "unreachable" || root.backendState === "error"
                 ? Commons.Color.urgent : Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.subtitle
        }
      }

      DetailPane {
        id: detailPane
        visible: root.detailOpen
        Layout.preferredWidth: root.detailOpen ? root.detailWidth : 0
        Layout.fillHeight: true
        txn: root.detailData
        loading: root.detailLoading
        error: root.detailError
        focusedPane: root.zone === "detail"
      }
    }

    Rectangle {
      Layout.fillWidth: true
      Layout.topMargin: Style.spacing.sm
      implicitHeight: Style.space(1)
      color: Commons.Color.menu.border
    }

    // Footer: key hints and position.
    RowLayout {
      Layout.fillWidth: true
      Layout.topMargin: Style.spacing.sm
      spacing: Style.spacing.lg * 2

      Text {
        Layout.fillWidth: true
        elide: Text.ElideRight
        text: "j/k move   Tab pane   / search   v views   f filter   s sort   i detail   b sidebar   ? help   r reload   q quit"
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
      Text {
        text: (rows.count > 0 ? (root.cursor + 1) : 0) + " / " + Query.fmtCount(root.total)
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }
  }

  // ------------------------------------------------------------------ overlays
  Toast {
    id: toastBar
    anchors.horizontalCenter: parent.horizontalCenter
    y: parent.height - height - Style.space(44)
  }

  Picker {
    id: picker
    anchors.fill: parent
    onChosen: function(key) { root.onPickerChosen(key) }
    onCancelled: root.onPickerCancelled()
    onDeleteRequested: function(key) { root.openViewDeleteConfirm(key, "views") }
    onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
  }

  InputPrompt {
    id: prompt
    anchors.fill: parent
    onSubmitted: function(text) { root.onPromptSubmitted(text) }
    onCancelled: root.onPromptCancelled()
    onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
  }

  HelpOverlay {
    id: help
    anchors.fill: parent
    onClosed: Qt.callLater(root.restoreFocus)
  }
}
