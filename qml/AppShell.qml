import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "state"
import "state/Query.js" as Query
import "keys/Keymap.js" as Keymap
import "state/Smart.js" as Smart
import "state/Insight.js" as Insight
import "state/Reports.js" as RepJs
import "common"
import "views"
import "common/Fuzzy.js" as Fuzzy
import "state/Search.js" as Search
import "state/Marks.js" as Marks
import "state/Tree.js" as Tree

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
  // Multi-select: committed marks {txid: true}. While visualOn, the rows from visualAnchor to the
  // cursor are marked too. markedSet is the effective set (the row highlight), markCount its size.
  property var marks: ({})
  property var markedSet: ({})
  property int markCount: 0
  property bool visualOn: false
  property int visualAnchor: -1
  property bool markAllPending: false
  readonly property int markCap: 5000

  // ------------------------------------------------------------------ query state
  // The one place table filters live. Query.buildQuery() is the only converter to a query string.
  property var filter: Query.emptyFilter()
  property var sort: ({ key: "date", dir: "desc" })
  // The last direction each column was sorted in. Keys 1-5 and the sort picker toggle that
  // column between its own ascending and descending order, so a column never lands in a third state.
  property var sortMemory: Query.defaultSortMemory()

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
  // Categories: the settings max depth, the category manager (overlay and cursor), and the
  // transactions/category a picker or prompt is about.
  property int maxDepth: 0
  property bool managerOpen: false
  property int mgrIndex: 0
  property int mgrCursorId: -1        // the manager cursor's node id, kept across reloads
  property string mgrStatus: ""
  property bool mgrStatusError: false
  property var catIds: []             // transactions the categorize picker acts on
  property bool smartFlow: false      // the category pick is for C (preview), not c (plain assign)
  // ------------------------------------------------------------------ categorize with preview (C) state
  property bool smartOpen: false
  property var smartCat: null         // {id, path}: the category the preview is for
  property var smartIds: []           // the rows the preview covers (<= Smart.PREVIEW_MAX)
  property var smartClusters: []      // /smart/preview clusters, response order
  property var smartSummary: null
  property var smartSelected: ({})    // cluster key -> bool
  property var smartExpanded: ({})    // cluster key -> true
  property var smartRows: ({})        // cluster key -> rows loaded so far
  property var smartLoadingByKey: ({})
  property real smartThreshold: 0.75  // requested (shown) threshold
  property bool smartOverwrite: false // requested (shown) overwrite
  property var smartShown: ({ threshold: 0.75, overwrite: false })  // what the clusters were computed with
  property bool smartRule: false      // "also make this a rule"
  property bool smartBusy: false      // a preview or apply request is in flight
  property string smartError: ""
  property int smartGen: 0            // bumped per preview request and on close; stale replies are dropped
  property var pendingCat: null       // {id, name, count, kids, into, intoName}: delete or merge target
  property int promptGen: 0           // bumped when a prompt opens or closes; stale replies are dropped
  property var mgrPendingParent: null // add: the new node's parent (null = top level)
  property int mgrPendingAfter: -1    // sibling add: the cursor node's index among its siblings
  property int mgrPendingId: -1       // rename target
  // ------------------------------------------------------------------ Phase 5: dashboard and insights (D)
  property bool dashOpen: false
  property int dashTab: 0                 // index into Insight.TABS
  property bool dashTransfers: false      // t: include transfer rows in the dashboard totals
  property var dashData: null             // GET /dashboard body, or null
  property string dashError: ""           // the backend's error envelope for /dashboard
  property var insightData: ({})          // tab name -> body (recurring / anomalies / transfers), or null
  property var insightErr: ({})           // tab name -> the backend's error envelope text, or ""
  property var dashCursors: ({})          // tab name -> row index of that list
  property int dashGen: 0
  // ------------------------------------------------------------------ Phase 5: review queue (R) and suggestions
  property bool reviewOpen: false
  property bool reportsOpen: false     // Phase 6: Reports overlay (P)
  property var reviewRows: []             // loaded GET /review rows: one per payee group, in queue order
  property string reviewMode: ""          // "suggest" | "frequency"
  property int reviewRemaining: 0         // groups the queue still holds
  property int reviewStart: 0             // remaining when the queue was opened (progress)
  property int reviewLastPage: 0          // size of the last GET /review page (more groups wait when it was full)
  property bool reviewLoading: false
  property string reviewError: ""
  property int reviewGen: 0               // bumped per reload and on close; stale replies are dropped
  property bool reviewDirty: false        // a group was applied: the table reloads when the queue closes
  property var reviewInFlight: ({})       // group key -> true while its apply request runs
  property var reviewPending: ({})        // group key -> {category_id, path}: what y applies
  property var reviewUnticked: ({})       // group key -> {id: true} for rows not ticked (all ticked by default)
  property var reviewExpanded: ({})       // group key -> true: Enter shows its rows
  property var reviewRowsByKey: ({})      // group key -> expanded rows loaded so far (POST /smart/rows)
  property var reviewRowsLoading: ({})    // group key -> true while its rows load
  property bool reviewPickFlow: false     // the next category the picker returns is the pending one
  property string reviewPickKey: ""       // the group that pick is for
  property var suggState: null            // {enabled, reason, accuracy} from /suggest/accuracy or POST /suggest; null = not loaded
  property string suggError: ""           // the backend's error envelope for the last suggestion request
  property var detailSugg: null           // the /suggest row for the detail pane's transaction, or null
  property int detailSuggGen: 0

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

  readonly property bool overlayOpen: picker.open || prompt.open || notesDlg.open || help.open || searchBar.open || managerOpen || smartOpen || dashOpen || reviewOpen || reportsOpen

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
      if (j && typeof j.max_depth === "number") root.maxDepth = j.max_depth
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
      syncFilterLabel()
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
    if (categoryList.length === 0)
      sideRows.append({ rowKind: "note", key: "", label: "No categories yet: press m", depth: 0, count: 0, sel: false, query: "" })
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
      if (xhr.status === 0) { setUnreachable(); markAllPending = false; seekEnd = false; restore = null; return }
      var j = parseJson(xhr.responseText)
      if (xhr.status === 400) {
        // The UI validates input before it gets here, so this is a bug report, not a user error.
        toast("filter rejected: " + ((j && j.error) || "bad request"))
        markAllPending = false
        seekEnd = false
        restore = null
        return
      }
      if (xhr.status !== 200 || !j || !j.rows) {
        backendState = "error"
        backendDetail = describeHttp(xhr)
        markAllPending = false
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
          recurring: !!r.recurring,
          notes: r.notes ? String(r.notes) : "",
          marked: !!markedSet[String(Number(r.id) || 0)]
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
      if (markAllPending) markAllStep()
      if (restore) settleSelection()
      else if (seekEnd) seekFinish()
      else maybePrefetch()
    })
  }

  // Rebuilds the table from the first page. The selected transaction is kept when it still
  // matches; otherwise the selection falls back to `fallbackIndex`.
  function reloadRows(fallbackIndex) {
    // A reload starts from the first page: a visual range and a pending mark-all end here.
    visualOn = false
    visualAnchor = -1
    markAllPending = false
    refreshMarks()
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
    marks = ({})     // marks belong to the filtered list they were made on
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
    sort = Query.nextSort(sort, key, sortMemory)
    sortMemory = Query.rememberSort(sortMemory, sort)
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
      { key: "notes", label: "Notes", detail: f.notes ? (f.notes === "*" ? "has a note" : "\"" + f.notes + "\"") : "any" },
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
    promptGen++
    prompt.closeOnSubmit = true
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
    } else if (ctx === "notesFilter") {
      prompt.title = "Notes filter"
      prompt.hint = "Text a note contains, or * for rows that have a note. Empty clears it."
      prompt.placeholder = "e.g. vacation, or *"
      prompt.validator = function(t) { var r = Query.parseNotesFilter(t); return r.error || "" }
      prompt.show(filter.notes)
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
    if (promptCtx.indexOf("report:") === 0) {
      var rerr = reportsView.promptResult(promptCtx.substring(7), text)
      if (rerr) prompt.fail(rerr)
      return
    }
    if (promptCtx === "newCat") { createAndAssign(text); return }
    if (promptCtx === "mgrAdd") { mgrCreate(text, false); return }
    if (promptCtx === "mgrSibling") { mgrCreate(text, true); return }
    if (promptCtx === "mgrRename") { mgrRename(text); return }
    var f = Object.assign({}, filter)
    if (promptCtx === "amount") {
      var a = Query.parseAmount(text)
      f.min = a.clear ? "" : a.min
      f.max = a.clear ? "" : a.max
      f.amountLabel = a.clear ? "" : a.label
    } else if (promptCtx === "text") {
      var t = Query.parseText(text)
      f.text = t.clear ? "" : t.text
    } else if (promptCtx === "notesFilter") {
      var nf = Query.parseNotesFilter(text)
      f.notes = nf.clear ? "" : nf.notes
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
    if (categoryPickerChosen(ctx, key)) return
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
      else if (key === "notes") openPrompt("notesFilter")
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
    reviewPickFlow = false
    if (isCategoryCtx(pickerCtx)) return
    if (pickerCtx === "views") return
    if (pickerCtx === "viewConfirm") { if (viewConfirmFrom === "views") openViewsPicker(); return }
    // A sub-picker opened from the filter menu goes back to that menu.
    if (pickerCtx !== "filter" && pickerCtx !== "sort") openFilterMenu()
  }

  function onPromptCancelled() {
    promptGen++
    if (promptCtx.indexOf("report:") === 0) return
    if (promptCtx === "newCat") { openCategorize(); return }
    if (promptCtx.indexOf("mgr") === 0) return
    if (promptCtx === "date") openDatePicker()
    else openFilterMenu()
  }

  // Give keys back to the shell after an overlay closes. Deferred: the closing overlay hides its
  // text field after onOpenChanged runs, which would otherwise take focus away again. Skipped if
  // another overlay reopened in the meantime (a sub-picker, for example).
  function restoreFocus() {
    if (picker.open || prompt.open || notesDlg.open || help.open || searchBar.open) return
    // An overlay's text field keeps the window's active focus after the overlay is hidden, so keys
    // would go to an invisible field. Drop it explicitly, then take focus back.
    var a = root.Window.activeFocusItem
    if (a && a !== root) a.focus = false
    picker.focus = false; prompt.focus = false; notesDlg.focus = false; help.focus = false
    searchBar.field.focus = false
    if (smartOpen) { smart.focusKeys(); return }
    if (reportsOpen) { reportsView.focusKeys(); return }
    if (reviewOpen) { review.focusKeys(); return }
    if (dashOpen) { dashView.focusKeys(); return }
    if (managerOpen) { manager.focusKeys(); return }
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
        if (j) { detailData = j; detailError = ""; loadDetailSuggestion(); return }
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

  // ------------------------------------------------------------------ errors and history
  // The backend's {message} for a failed request (a 409 says why, a 400 names the bad field).
  function xhrMessage(xhr) {
    if (xhr.status === 0) return "backend not responding (port " + Backend.port + ")"
    var j = parseJson(xhr.responseText)
    if (j && j.message) return String(j.message)
    return describeHttp(xhr)
  }

  // u / C-r and :undo / :redo. The server names the change; a 409 (nothing to undo, conflict) says why.
  function history(kind) {
    Backend.request("POST", "/" + kind, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { toast(xhrMessage(xhr)); return }
      toast((kind === "undo" ? "Undid: " : "Redid: ") + String(j.label || ""))
      afterChange()
    })
  }

  // :journal. A read-only picker of the last 20 batches, newest first; undone ones are marked.
  function openJournal() {
    Backend.request("GET", "/journal?limit=20", function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { toast(xhrMessage(xhr)); return }
      pickerCtx = "journal"
      picker.title = "Journal: the last 20 changes, newest first"
      picker.hint = (j.can_undo ? "u undoes: " + String(j.next_undo_label || "") : "nothing to undo")
                    + (j.can_redo ? "   C-r redoes: " + String(j.next_redo_label || "") : "")
      picker.emptyText = "no changes yet"
      picker.deletable = false
      picker.entries = Tree.journalEntries(j.rows || [])
      picker.currentKey = ""
      picker.show()
    })
  }

  // Reloads the table and the sidebar after any change to transactions or categories.
  function afterChange() {
    loadFacets()
    reloadRows(cursor)
    if (reviewOpen) { reviewReload(false); loadSuggestAccuracy() }
  }

  // A category chip shows the category's current name (it may have been renamed in the manager).
  function syncFilterLabel() {
    if (filter.category === "" || filter.category === "none") return
    var c = Tree.rowOf(categoryList, filter.category)
    if (!c || String(c.name) === filter.categoryLabel) return
    var f = Object.assign({}, filter)
    f.categoryLabel = String(c.name)
    filter = f
  }

  // ------------------------------------------------------------------ table: marks (multi-select)
  function loadedIds() {
    var out = []
    for (var i = 0; i < rows.count; i++) out.push(Number(rows.get(i).txid))
    return out
  }

  // Recomputes the effective marks: the header count and the highlight on every loaded row.
  function refreshMarks() {
    var ids = Marks.effectiveIds(loadedIds(), marks, visualOn, visualAnchor, cursor)
    var set = {}
    for (var k = 0; k < ids.length; k++) set[String(ids[k])] = true
    markedSet = set
    markCount = ids.length
    for (var i = 0; i < rows.count; i++) {
      var want = !!set[String(rows.get(i).txid)]
      if (rows.get(i).marked !== want) rows.setProperty(i, "marked", want)
    }
  }

  // Ends visual mode; the range becomes committed marks.
  function endVisual() {
    if (visualOn) marks = Marks.addAll({}, Marks.effectiveIds(loadedIds(), marks, true, visualAnchor, cursor))
    visualOn = false
    visualAnchor = -1
    refreshMarks()
  }

  // V: start a visual range at the cursor; V again ends it and keeps the marks.
  function toggleVisual() {
    if (visualOn) { endVisual(); return }
    if (currentTxid() < 0) { toast("no row under the cursor"); return }
    visualOn = true
    visualAnchor = cursor
    refreshMarks()
  }

  // Space: mark or unmark the cursor row, then move down. A visual range is committed first.
  function toggleMarkCursor() {
    if (visualOn) endVisual()
    var id = currentTxid()
    if (id < 0) return
    marks = Marks.toggle(marks, id)
    refreshMarks()
    moveRow(1)
  }

  // Esc in the table: end visual mode, else clear the marks. Esc never quits; q does.
  function escapeTable() {
    if (visualOn) { endVisual(); return }
    if (markCount > 0) { marks = ({}); refreshMarks(); return }
  }

  // C-a: mark every row that matches the filters. Pages load until every matching row (up to the
  // cap) is in; markAllStep runs again after each page.
  function markAll() {
    if (visualOn) endVisual()
    if (summaryKnown && total === 0) { toast("no rows match these filters"); return }
    markAllPending = true
    markAllStep()
  }

  function markAllStep() {
    if (!markAllPending) return
    var goal = Math.min(total, markCap)
    if (rows.count < goal) { loadPage(); return }
    markAllPending = false
    var cap = Marks.capIds(loadedIds(), markCap)
    marks = Marks.addAll(marks, cap.ids)
    refreshMarks()
    toast(cap.capped
          ? "marked the first " + Query.fmtCount(markCap) + " of " + Query.fmtCount(total) + " matching rows (cap)"
          : "marked " + Query.fmtCount(cap.ids.length) + " rows")
  }

  // ------------------------------------------------------------------ categorize (c)
  // Target: the marked rows, else the cursor row. The picker is the generic Picker over the tree.
  function startCategorize(withPreview) {
    if (visualOn) endVisual()
    var list = Marks.effectiveIds(loadedIds(), marks, false, -1, -1)
    if (list.length === 0) {
      var id = currentTxid()
      if (id < 0) { toast("no transaction under the cursor"); return }
      list = [id]
    }
    catIds = list
    smartFlow = !!withPreview
    reviewPickFlow = false
    openCategorize()
  }

  // ------------------------------------------------------------------ notes (e)
  // Target: the marked rows, else the cursor row. One row prefills its current note.
  property var noteIds: []

  function startNotes() {
    var list = Marks.effectiveIds(loadedIds(), marks, visualOn, visualAnchor, cursor)
    var initial = ""
    if (list.length === 0) {
      var id = currentTxid()
      if (id < 0) { toast("no transaction under the cursor"); return }
      list = [id]
    }
    noteGen++
    noteIds = list
    if (list.length === 1) {
      var i = indexOfTxid(list[0])
      var r = i >= 0 ? rows.get(i) : null
      if (r) initial = String(r.notes || "")
      notesDlg.title = "Transaction note"
      notesDlg.payee = r ? (r.payee || "(no payee)") : ""
      notesDlg.detail = r ? [r.date, Query.fmtAmountRow(r.amount, r.currency), r.account,
                             r.category].filter(function(x) { return !!x }).join("   \u00b7   ") : ""
      notesDlg.footnote = ""
    } else {
      notesDlg.title = "Note for " + Query.fmtCount(list.length) + " transactions"
      notesDlg.payee = ""
      notesDlg.detail = ""
      notesDlg.footnote = "This replaces the note on every marked row. Empty removes their notes."
    }
    notesDlg.show(initial)
  }

  property int noteGen: 0

  function saveNotes(text) {
    var p = Query.parseNote(text)
    if (p.error) { notesDlg.fail(p.error); return }
    var gen = noteGen
    var ids = noteIds
    var value = p.clear ? null : p.note
    notesDlg.busy = true
    var done = function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (gen !== noteGen || !notesDlg.open) return
      if (!j) { notesDlg.fail(xhrMessage(xhr)); return }
      notesDlg.close()
      toast(String(j.label || "note saved") + "   u to undo")
      if (ids.length > 1) { marks = ({}); visualOn = false; visualAnchor = -1; refreshMarks() }
      afterChange()
      loadDetail()
    }
    if (ids.length === 1) Backend.json("PUT", "/transactions/" + ids[0] + "/notes", { notes: value }, done)
    else Backend.json("POST", "/transactions/notes", { ids: ids, notes: value }, done)
  }

  function openCategorize() {
    var n = catIds.length
    pickerCtx = "categorize"
    picker.title = n === 1 ? "Categorize this transaction" : "Categorize " + Query.fmtCount(n) + " transactions"
    picker.hint = categoryList.length === 0
        ? "No categories yet: choose + New category, or press m for the category manager."
        : "Type to filter. Full paths match: hm finds Home \u203a Maintenance."
    picker.emptyText = "no matching category"
    picker.deletable = false
    picker.entries = Tree.pickerEntries(categoryList, {
      uncategorized: categoryList.length > 0 && !smartFlow, uncatCount: uncategorizedCount, newEntry: true })
    picker.currentKey = ""
    picker.show()
  }

  // A transaction-category assignment. The table reloads keeping the cursor's transaction.
  function assignCategory(ids, catId) {
    if (ids.length === 0) return
    Backend.json("POST", "/transactions/categorize", { ids: ids, category_id: catId }, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { toast(xhrMessage(xhr)); return }
      toast(String(j.label || "categorized") + "   u to undo")
      marks = ({})
      visualOn = false
      visualAnchor = -1
      refreshMarks()
      afterChange()
    })
  }

  // The category is chosen: a plain assignment for c, the preview for C.
  function finishCategorize(ids, catId) {
    if (reviewPickFlow) reviewPendingPick(catId)
    else if (smartFlow) smartStart(catId)
    else assignCategory(ids, catId)
  }

  // ------------------------------------------------------------------ categorize with preview (C)
  // C: the marked rows (or the cursor row), the category picker, then POST /smart/preview and the
  // preview overlay. Nothing is written until smartApply(); Esc writes nothing. A reply is dropped
  // when a newer request (or a close) superseded it; an apply reply always lands, because the write
  // already happened.
  function smartStart(catId) {
    var ids = catIds
    if (ids.length === 0) return
    if (ids.length > Smart.PREVIEW_MAX) {
      toast("the preview covers at most " + Query.fmtCount(Smart.PREVIEW_MAX) + " rows; "
            + Query.fmtCount(ids.length) + " are marked: mark fewer")
      return
    }
    smartIds = ids
    smartCat = { id: catId, path: Tree.pathOf(categoryList, String(catId)) }
    smartClusters = []
    smartSummary = null
    smartSelected = ({})
    smartExpanded = ({})
    smartRows = ({})
    smartLoadingByKey = ({})
    smartRule = false
    smartThreshold = 0.75
    smartOverwrite = false
    smartShown = { threshold: 0.75, overwrite: false }
    smartError = ""
    smartOpen = true
    smartRequest(false)
    Qt.callLater(smart.focusKeys)
  }

  // POST /smart/preview with the requested threshold and overwrite. resetConflicts: the overwrite
  // toggle changed, so conflict clusters go back to their new default. Other clusters keep their
  // selection by label.
  function smartRequest(resetConflicts) {
    var g = ++smartGen
    smartBusy = true
    smartError = ""
    Backend.json("POST", "/smart/preview", {
      ids: smartIds, category_id: smartCat.id, threshold: smartThreshold, overwrite: smartOverwrite
    }, function(xhr) {
      if (g !== smartGen) return
      smartBusy = false
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j || !j.clusters) {
        // Keep the clusters on screen; show the error and put the controls back to what they show.
        smartError = Smart.errorMessage(j, xhrMessage(xhr))
        smartThreshold = smartShown.threshold
        smartOverwrite = smartShown.overwrite
        return
      }
      smartShown = { threshold: Smart.clampThreshold(j.threshold), overwrite: smartOverwrite }
      smartSelected = Smart.retainSelection(smartSelected, j.clusters, resetConflicts)
      smartClusters = j.clusters
      smartSummary = j.summary || null
      smartThreshold = smartShown.threshold
      // Expansion survives a re-query for clusters whose key is still there; their rows reload.
      var nk = Smart.clusterKeys(j.clusters)
      var keep = {}
      nk.forEach(function(k) { if (smartExpanded[k]) keep[k] = true })
      smartExpanded = keep
      smartRows = ({})
      smartLoadingByKey = ({})
      Object.keys(keep).forEach(function(k) { smartLoadRows(k) })
      if (j.category && j.category.path) smartCat = { id: smartCat.id, path: String(j.category.path) }
    })
  }

  // Overlay keys (smart scope, minus the cursor moves the overlay does itself).
  function smartAct(id) {
    var it = smart.currentItem()
    if (id === "smart.toggle") { if (it) smartSelected = Smart.toggled(smartSelected, it.ckey) }
    else if (id === "smart.all") smartSelected = Smart.setAll(smartClusters, true)
    else if (id === "smart.none") smartSelected = Smart.setAll(smartClusters, false)
    else if (id === "smart.expand") { if (it) smartExpand(it) }
    else if (id === "smart.thresholdDown") smartStep(-1)
    else if (id === "smart.thresholdUp") smartStep(1)
    else if (id === "smart.overwrite") { smartOverwrite = !smartOverwrite; smartRequest(true) }
    else if (id === "smart.rule") smartRule = !smartRule
    else if (id === "smart.apply") smartApply()
    else if (id === "smart.cancel") smartClose()
  }

  function smartStep(dir) {
    var t = Smart.stepThreshold(smartThreshold, dir)
    if (t === smartThreshold) {
      toast("threshold is at " + Smart.thresholdPercent(t) + " (" + (dir > 0 ? "top" : "bottom") + ")")
      return
    }
    smartThreshold = t
    smartRequest(false)
  }

  // Enter on a cluster: expand or collapse it (loading its first page). On "more": the next page.
  function smartExpand(it) {
    var k = it.ckey
    if (it.kind === "more") { smartLoadRows(k); return }
    var e = Object.assign({}, smartExpanded)
    if (e[k]) { delete e[k]; smartExpanded = e; return }
    e[k] = true
    smartExpanded = e
    smartLoadRows(k)
  }

  // POST /smart/rows for the next 50 ids of a cluster (lazy; the cluster's rows arrive in order).
  function smartLoadRows(k) {
    var c = Smart.clusterByKey(smartClusters, Smart.clusterKeys(smartClusters), k)
    if (!c || smartLoadingByKey[k]) return
    var have = (smartRows[k] || []).length
    var ids = Smart.nextPageIds(c.txn_ids, have)
    if (ids.length === 0) return
    var g = smartGen
    var l = Object.assign({}, smartLoadingByKey)
    l[k] = true
    smartLoadingByKey = l
    Backend.json("POST", "/smart/rows", { ids: ids }, function(xhr) {
      if (g !== smartGen) return
      var done = Object.assign({}, smartLoadingByKey)
      delete done[k]
      smartLoadingByKey = done
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j || !j.rows) { smartError = Smart.errorMessage(j, xhrMessage(xhr)); return }
      var m = Object.assign({}, smartRows)
      m[k] = (smartRows[k] || []).concat(j.rows)
      smartRows = m
    })
  }

  // y: POST /smart/apply with the selected clusters' rows, one batch. The toast names the change.
  function smartApply() {
    if (smartBusy || smartClusters.length === 0) return
    var ids = Smart.selectedTxnIds(smartClusters, smartSelected)
    if (ids.length === 0) { smartError = "nothing selected: Space picks a cluster, a selects all"; return }
    if (ids.length > Smart.APPLY_MAX) {
      smartError = Query.fmtCount(ids.length) + " rows selected; the limit is "
                   + Query.fmtCount(Smart.APPLY_MAX) + ". Deselect some."
      return
    }
    smartBusy = true
    smartError = ""
    Backend.json("POST", "/smart/apply", {
      category_id: smartCat.id, txn_ids: ids, overwrite: smartShown.overwrite,
      make_rule: smartRule ? {} : null
    }, function(xhr) {
      smartBusy = false
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j || !j.ok) {
        var msg = Smart.errorMessage(j, xhrMessage(xhr))
        if (smartOpen) smartError = msg
        else toast(msg)
        return
      }
      var note = String(j.label || "categorized")
      if (Number(j.skipped_conflicts) > 0) note += "  \u00b7  " + Query.fmtCount(Number(j.skipped_conflicts)) + " conflicts skipped"
      toast(note + "   u to undo")
      smartClose()
      marks = ({})
      visualOn = false
      visualAnchor = -1
      refreshMarks()
      afterChange()
    })
  }

  // Esc (and after apply): close without writing. Any preview reply still in flight is dropped.
  function smartClose() {
    smartGen++
    smartOpen = false
    smartBusy = false
  }

  // "+ New category": the typed path is resolved; missing levels are created one by one, then the
  // transactions are assigned. A 409 from the server (max depth) is shown inline.
  function openNewCategory() {
    openMgrPrompt("newCat", "New category",
                  "Type a path from the top: Home/Pets/Dogs. Missing levels are created. Max depth " + maxDepth + ".",
                  "Home/Pets/Dogs", "", true)
  }

  function createAndAssign(text) {
    var p = Tree.parsePath(text)
    if (p.error) { prompt.fail(p.error); return }
    var r = Tree.resolvePath(categoryList, p.segments)
    var ids = catIds
    if (r.missing.length === 0) { prompt.close(); finishCategorize(ids, r.id); return }
    var gen = promptGen
    prompt.busy = true
    createChain(r.parentId, r.missing, 0, function(err, newId) {
      if (gen !== promptGen) { loadFacets(); return }      // cancelled meanwhile: keep the new levels, assign nothing
      if (err !== null) { prompt.fail(err); loadFacets(); return }
      prompt.close()
      finishCategorize(ids, newId)
    })
  }

  // Creates names[i] under pid, then the next one under it, and so on. done(err, lastId).
  function createChain(pid, names, i, done) {
    Backend.json("POST", "/categories", { parent_id: pid, name: names[i] }, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { done(xhrMessage(xhr), null); return }
      if (i + 1 >= names.length) { done(null, Number(j.id)); return }
      createChain(Number(j.id), names, i + 1, done)
    })
  }

  // Closes the prompt on success. On failure the message goes inline when the prompt is still the
  // one that asked, else to the manager's status line.
  function settlePrompt(gen, ok, msg) {
    if (gen === promptGen && prompt.open) {
      if (ok) prompt.close()
      else prompt.fail(msg)
      return
    }
    if (!ok) mgrError(msg)
  }

  // Pickers opened by the category features. Returns true when the key was theirs.
  function categoryPickerChosen(ctx, key) {
    if (ctx === "categorize") {
      if (key === "new") openNewCategory()
      else if (reviewPickFlow) reviewPendingPick(key === "none" ? null : Number(key))
      else if (smartFlow) smartStart(Number(key))
      else assignCategory(catIds, key === "none" ? null : Number(key))
      return true
    }
    if (ctx === "mgrDelete") {
      if (key === "uncat") deleteCategory(pendingCat.id, null)
      else if (key === "move") openReassign()
      return true
    }
    if (ctx === "reassign") {
      deleteCategory(pendingCat.id, key === "none" ? null : Number(key))
      return true
    }
    if (ctx === "mergeTarget") {
      pendingCat.into = Number(key)
      pendingCat.intoName = Tree.pathOf(categoryList, key)
      openMergeConfirm()
      return true
    }
    if (ctx === "mgrMerge") {
      if (key === "yes") mergeCategory(pendingCat.id, pendingCat.into)
      return true
    }
    return ctx === "journal"
  }

  function isCategoryCtx(c) {
    return ["categorize", "mgrDelete", "reassign", "mergeTarget", "mgrMerge", "journal"].indexOf(c) >= 0
  }

  // ------------------------------------------------------------------ category manager (m)
  function mgrNode() {
    return mgrIndex >= 0 && mgrIndex < categoryList.length ? categoryList[mgrIndex] : null
  }

  // Keeps the cursor on the same node id across reloads; else the nearest row.
  function mgrSync() {
    mgrIndex = Tree.restoreIndex(categoryList, mgrCursorId, mgrIndex)
    if (mgrIndex < 0) mgrIndex = 0
    var cur = mgrNode()
    mgrCursorId = cur ? Number(cur.id) : -1
  }

  function mgrMove(d) {
    if (categoryList.length === 0) return
    mgrIndex = Math.max(0, Math.min(categoryList.length - 1, mgrIndex + d))
    mgrCursorId = Number(categoryList[mgrIndex].id)
  }

  function mgrNote(msg) { mgrStatus = msg; mgrStatusError = false }
  function mgrError(msg) { mgrStatus = msg; mgrStatusError = true }

  function openManager() {
    if (overlayOpen) return
    mgrStatus = ""
    mgrStatusError = false
    loadFacets()
    managerOpen = true
    manager.focusKeys()
  }

  function closeManager() { managerOpen = false }

  // Keys from the manager's scope. Actions on a node do nothing on an empty tree.
  function mgrAct(id) {
    var cur = mgrNode()
    switch (id) {
      case "mgr.down": mgrMove(1); break
      case "mgr.up": mgrMove(-1); break
      case "mgr.close": closeManager(); break
      case "mgr.add": mgrOpenAdd(false); break
      case "mgr.addSibling": mgrOpenAdd(true); break
      case "mgr.rename": if (cur) mgrOpenRename(cur); break
      case "mgr.outdent": if (cur) mgrOutdent(cur); break
      case "mgr.indent": if (cur) mgrIndent(cur); break
      case "mgr.shiftUp": if (cur) mgrShift(cur, -1); break
      case "mgr.shiftDown": if (cur) mgrShift(cur, 1); break
      case "mgr.kind": if (cur) mgrPost("/categories/" + cur.id + "/kind", { kind: Tree.kindNext(cur.kind) }, null, cur.id); break
      case "mgr.delete": if (cur) openDeleteAsk(cur); break
      case "mgr.merge": if (cur) openMergeTarget(cur); break
      case "mgr.filter": if (cur) mgrFilter(cur); break
    }
  }

  function openMgrPrompt(ctx, title, hint, placeholder, initial, pathMode) {
    promptCtx = ctx
    promptGen++
    prompt.title = title
    prompt.hint = hint
    prompt.placeholder = placeholder
    prompt.closeOnSubmit = false
    prompt.validator = pathMode
        ? function(t) { var r = Tree.parsePath(t); return r.error || "" }
        : function(t) { var r = Tree.parseName(t); return r.error || "" }
    prompt.show(initial)
  }

  // a: child of the cursor node (top level on an empty tree). A: sibling after the cursor node.
  function mgrOpenAdd(sibling) {
    var cur = mgrNode()
    // A child goes under the cursor node; a sibling goes beside it, under the cursor's parent.
    mgrPendingParent = cur ? (sibling ? Tree.pidOf(cur) : Number(cur.id)) : null
    mgrPendingAfter = sibling && cur ? Tree.siblingIndex(categoryList, cur.id) : -1
    var where = cur ? (sibling ? "After '" : "Under '") + cur.name + "'." : "Top level."
    openMgrPrompt(sibling ? "mgrSibling" : "mgrAdd", sibling ? "Add a sibling" : "Add a child",
                  where + " Names cannot contain /.", "name", "", false)
  }

  function mgrCreate(text, sibling) {
    var p = Tree.parseName(text)
    if (p.error) { prompt.fail(p.error); return }
    var gen = promptGen
    var parent = mgrPendingParent
    var after = sibling ? mgrPendingAfter : -1
    prompt.busy = true
    Backend.json("POST", "/categories", { parent_id: parent, name: p.name }, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { settlePrompt(gen, false, xhrMessage(xhr)); return }
      var newId = Number(j.id)
      settlePrompt(gen, true, "")
      mgrCursorId = newId
      if (after >= 0) {
        // The server appends the new node last; move it to sit right after the cursor node.
        Backend.json("POST", "/categories/" + newId + "/move", { parent_id: parent, position: after + 1 }, function(x2) {
          var j2 = x2.status === 200 ? parseJson(x2.responseText) : null
          if (!j2) mgrError(xhrMessage(x2))
          else mgrNote(String(j2.label || ""))
          afterChange()
        })
        return
      }
      mgrNote(String(j.label || ""))
      afterChange()
    })
  }

  function mgrOpenRename(cur) {
    mgrPendingId = Number(cur.id)
    openMgrPrompt("mgrRename", "Rename", "Rename '" + cur.name + "'.", "name", String(cur.name), false)
  }

  function mgrRename(text) {
    var p = Tree.parseName(text)
    if (p.error) { prompt.fail(p.error); return }
    var gen = promptGen
    var id = mgrPendingId
    prompt.busy = true
    Backend.json("POST", "/categories/" + id + "/rename", { name: p.name }, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { settlePrompt(gen, false, xhrMessage(xhr)); return }
      settlePrompt(gen, true, "")
      mgrCursorId = id
      mgrNote(String(j.label || ""))
      afterChange()
    })
  }

  // POST helper for the manager: shows the server's label, then reloads (or runs done).
  function mgrPost(path, body, done, cursorId) {
    if (cursorId !== undefined) mgrCursorId = Number(cursorId)
    Backend.json("POST", path, body, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { mgrError(xhrMessage(xhr)); return }
      mgrNote(String(j.label || ""))
      if (done) done(j)
      else afterChange()
    })
  }

  function mgrOutdent(cur) {
    var t = Tree.outdentTarget(categoryList, cur.id)
    if (t === null) { mgrError("already at the top level"); return }
    mgrPost("/categories/" + cur.id + "/move", { parent_id: t.parent_id, position: t.position }, null, cur.id)
  }

  function mgrIndent(cur) {
    var t = Tree.indentTarget(categoryList, cur.id)
    if (t === null) { mgrError("no previous sibling to indent under"); return }
    mgrPost("/categories/" + cur.id + "/move", { parent_id: t.parent_id, position: t.position }, null, cur.id)
  }

  function mgrShift(cur, d) {
    if (!Tree.canShift(categoryList, cur.id, d)) {
      mgrError(d < 0 ? "already first among its siblings" : "already last among its siblings")
      return
    }
    mgrPost("/categories/" + cur.id + "/shift", { delta: d }, null, cur.id)
  }

  // Enter: filter the table by this category and close the manager.
  function mgrFilter(cur) {
    closeManager()
    var f = Object.assign({}, filter)
    f.category = String(cur.id)
    f.categoryLabel = String(cur.name)
    zone = "table"
    applyFilter(f)
  }

  // d: asks what happens to the node's own transactions. Subcategories always move up a level.
  function openDeleteAsk(cur) {
    var id = Number(cur.id)
    var kids = Tree.childrenOf(categoryList, id).length
    pendingCat = { id: id, name: String(cur.name), count: Number(cur.count) || 0, kids: kids, into: null, intoName: "" }
    pickerCtx = "mgrDelete"
    picker.title = "Delete '" + pendingCat.name + "'?"
    picker.hint = Query.fmtCount(pendingCat.count) + " transaction(s) are in it directly"
                  + (kids ? "; its " + kids + (kids === 1 ? " subcategory moves" : " subcategories move") + " up one level" : "")
                  + ". Transactions in subcategories stay where they are."
    picker.emptyText = ""
    picker.deletable = false
    picker.entries = [
      { key: "uncat", label: "Delete; its transactions go to Uncategorized", detail: "" },
      { key: "move", label: "Delete; its transactions go to a category I pick\u2026", detail: "" },
      { key: "keep", label: "Keep it", detail: "" }
    ]
    picker.currentKey = "keep"
    picker.show()
  }

  function openReassign() {
    pickerCtx = "reassign"
    picker.title = "Move its transactions to..."
    picker.hint = "Pick where the " + Query.fmtCount(pendingCat.count) + " transaction(s) of '" + pendingCat.name + "' go."
    picker.emptyText = "no matching category"
    picker.deletable = false
    picker.entries = Tree.pickerEntries(categoryList, {
      uncategorized: true, uncatCount: uncategorizedCount, exclude: Tree.subtreeIds(categoryList, pendingCat.id) })
    picker.currentKey = "none"
    picker.show()
  }

  function deleteCategory(id, reassignTo) {
    var gone = Number(id)
    var nb = Tree.neighbourAfterDelete(categoryList, gone)
    mgrPost("/categories/" + gone + "/delete", { reassign_to: reassignTo },
            function() { afterTreeChange(gone, null) }, nb === null ? -1 : nb)
  }

  // M: pick the target (the node and its subtree are not offered), then confirm.
  function openMergeTarget(cur) {
    var id = Number(cur.id)
    pendingCat = { id: id, name: String(cur.name), count: Number(cur.count) || 0, kids: 0, into: null, intoName: "" }
    pickerCtx = "mergeTarget"
    picker.title = "Merge '" + pendingCat.name + "' into..."
    picker.hint = "Its transactions and subcategories move into the target; same-name subcategories merge."
    picker.emptyText = "no matching category"
    picker.deletable = false
    picker.entries = Tree.pickerEntries(categoryList, { exclude: Tree.subtreeIds(categoryList, id) })
    picker.currentKey = ""
    picker.show()
  }

  function openMergeConfirm() {
    pickerCtx = "mgrMerge"
    picker.title = "Merge '" + pendingCat.name + "' into '" + pendingCat.intoName + "'?"
    picker.hint = Query.fmtCount(pendingCat.count) + " transaction(s) directly in it move; '" + pendingCat.name + "' is deleted."
    picker.emptyText = ""
    picker.deletable = false
    picker.entries = [ { key: "yes", label: "Merge", detail: "" }, { key: "no", label: "Cancel", detail: "" } ]
    picker.currentKey = "no"
    picker.show()
  }

  function mergeCategory(src, dst) {
    mgrPost("/categories/" + src + "/merge", { into: dst }, function() { afterTreeChange(src, dst) }, dst)
  }

  // After a delete or merge the table reloads. A filter on the removed category moves to the merge
  // target, or is cleared after a delete.
  function afterTreeChange(gone, into) {
    loadFacets()
    if (String(filter.category) !== String(gone)) { reloadRows(cursor); return }
    var f = Object.assign({}, filter)
    if (into === null) { f.category = ""; f.categoryLabel = "" }
    else {
      var t = Tree.rowOf(categoryList, into)
      f.category = String(into)
      f.categoryLabel = t ? String(t.name) : ""
    }
    zone = "table"
    applyFilter(f)
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
    if (sortChanged) {
      sort = { key: sortObj.key, dir: sortObj.dir }
      sortMemory = Query.rememberSort(sortMemory, sort)
    }
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
    else if (pc.cmd === "undo") { closeSearch(false); history("undo") }
    else if (pc.cmd === "redo") { closeSearch(false); history("redo") }
    else if (pc.cmd === "journal") { closeSearch(false); openJournal() }
    else if (pc.cmd === "reports") { closeSearch(false); openReports() }
    else if (pc.cmd === "report") { closeSearch(false); openReports(pc.arg) }
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
  // ------------------------------------------------------------------ Phase 5: insights, review, suggestions
  // AppShell makes every request and owns every piece of state; the overlays and DetailPane render
  // the properties and emit ids back here.

  // The backend's error envelope shown verbatim ("error: message"), else the HTTP status.
  function envelopeText(xhr) {
    if (xhr.status === 0) return "backend not responding (port " + Backend.port + ")"
    var j = parseJson(xhr.responseText)
    if (j && (j.error || j.message)) return (j.error ? String(j.error) + ": " : "") + String(j.message || "")
    return describeHttp(xhr)
  }

  // The suggest gate: GET /suggest/accuracy -> suggState {enabled, reason, accuracy}.
  function loadSuggestAccuracy() {
    Backend.request("GET", "/suggest/accuracy", function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { suggError = envelopeText(xhr); return }
      suggError = ""
      suggState = { enabled: j.enabled === true, reason: j.reason || null, accuracy: j.accuracy || null }
    })
  }

  // ---- dashboard and insights
  function openDashboard() {
    if (overlayOpen) return
    dashTab = 0
    dashOpen = true
    loadDashboard()
  }

  function closeDashboard() { dashOpen = false }

  // ---- reports (Phase 6): the overlay owns its requests; this is the entry, the text prompt bridge.
  function openReports(newName) {
    if (overlayOpen) return
    reportsOpen = true
    reportsView.currentQuery = filter.slash
    reportsView.show(newName)
  }

  function openReportPrompt(ctx, title, hint, placeholder, initial) {
    promptCtx = "report:" + ctx
    promptGen++
    prompt.title = title
    prompt.hint = hint
    prompt.placeholder = placeholder
    prompt.closeOnSubmit = true
    prompt.validator = function(t) {
      if (ctx === "name") return t.trim() === "" ? "name is required" : ""
      if (ctx === "time") return RepJs.timeError(t.trim())
      if (ctx === "recipients") return RepJs.recipientsError(RepJs.parseRecipients(t))
      return ""
    }
    prompt.show(initial)
  }

  function loadDashboard() {
    var g = ++dashGen
    var path = "/dashboard?months=12&exclude_transfers=" + (dashTransfers ? "false" : "true")
    Backend.request("GET", path, function(xhr) {
      if (g !== dashGen) return
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { dashData = null; dashError = envelopeText(xhr); return }
      dashData = j
      dashError = ""
    })
  }

  function insightPath(name) {
    if (name === "recurring") return "/insights/recurring"
    if (name === "anomalies") return "/insights/anomalies"
    return "/insights/transfers"
  }

  function loadInsight(name) {
    Backend.request("GET", insightPath(name), function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      var d = Object.assign({}, insightData)
      var e = Object.assign({}, insightErr)
      if (!j) { d[name] = null; e[name] = envelopeText(xhr) }
      else { d[name] = j; e[name] = "" }
      insightData = d
      insightErr = e
    })
  }

  function switchDashTab(i) {
    dashTab = i
    var name = Insight.TABS[i]
    if (name !== "dashboard") loadInsight(name)
  }

  // The rows of a list tab (recurring series, anomalies, transfer pairs).
  function dashItems(name) {
    var b = insightData[name]
    if (!b) return []
    if (name === "recurring") return b.series || []
    if (name === "anomalies") return b.items || []
    return b.pairs || []
  }

  function moveDashCursor(dir) {
    var name = Insight.TABS[dashTab]
    if (name === "dashboard") return
    var c = Object.assign({}, dashCursors)
    c[name] = Insight.moveIndex(c[name] || 0, dir, dashItems(name).length)
    dashCursors = c
  }

  // Enter: the table jumps to the rows of this series, anomaly or pair, then the overlay closes.
  function jumpFromDash() {
    var name = Insight.TABS[dashTab]
    var items = dashItems(name)
    if (items.length === 0) return
    var row = items[Insight.clamp((dashCursors[name] || 0), items.length)]
    var q = Insight.jumpQueryFor(name, row)
    if (q === "") { toast("nothing to jump to"); return }
    closeDashboard()
    applySlash(q, null)
  }

  function dashAct(id) {
    if (id === "dash.close") closeDashboard()
    else if (id === "dash.next") switchDashTab(Insight.nextTab(dashTab, 1))
    else if (id === "dash.prev") switchDashTab(Insight.nextTab(dashTab, -1))
    else if (id === "dash.down") moveDashCursor(1)
    else if (id === "dash.up") moveDashCursor(-1)
    else if (id === "dash.jump") jumpFromDash()
    else if (id === "dash.transfers") { dashTransfers = !dashTransfers; loadDashboard() }
  }

  // ---- review queue: one entry per payee group (the amendment in docs/PHASE5_CONTRACT.md).
  // y categorizes the ticked rows of the group as its pending category (one undoable request); the
  // group then leaves the queue. The view owns the cursor and movement; reviewAct handles the rest.
  function groupAt() {
    var it = review.cur
    return it ? Insight.groupByKey(reviewRows, it.gkey) : null
  }

  function withoutKey(map, k) {
    var m = Object.assign({}, map)
    delete m[k]
    return m
  }

  function openReview() {
    if (overlayOpen) return
    reviewOpen = true
    reviewDirty = false
    reviewRows = []
    reviewError = ""
    reviewPending = ({})
    reviewUnticked = ({})
    reviewExpanded = ({})
    reviewRowsByKey = ({})
    reviewRowsLoading = ({})
    reviewPickFlow = false
    loadSuggestAccuracy()
    reviewReload(true)
  }

  function closeReview() {
    reviewOpen = false
    reviewGen++
    reviewPickFlow = false
    reviewRowsLoading = ({})
    if (reviewDirty) { reviewDirty = false; afterChange() }
  }

  // GET /review: the groups, as many as are loaded already (50 at least, 200 at most). fresh: this is
  // the first load of the queue (the progress bar starts here). Per-group state of groups that left
  // the queue is dropped; expanded groups that stay reload their rows.
  function reviewReload(fresh) {
    var g = ++reviewGen
    var want = Math.min(Insight.REVIEW_MAX, Math.max(Insight.REVIEW_PAGE, reviewRows.length))
    reviewLoading = true
    Backend.request("GET", "/review?limit=" + want + "&offset=0", function(xhr) {
      if (g !== reviewGen) return
      reviewLoading = false
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j || !j.rows) { reviewRows = []; reviewError = envelopeText(xhr); return }
      reviewError = ""
      reviewMode = String(j.mode || "")
      reviewRemaining = Number(j.remaining) || 0
      if (fresh) reviewStart = reviewRemaining
      reviewLastPage = j.rows.length
      reviewRows = j.rows
      var keys = Insight.groupKeys(j.rows)
      reviewPending = Insight.pruneTo(reviewPending, keys)
      reviewUnticked = Insight.pruneTo(reviewUnticked, keys)
      reviewExpanded = Insight.pruneTo(reviewExpanded, keys)
      reviewRowsByKey = ({})
      reviewRowsLoading = ({})
      Object.keys(reviewExpanded).forEach(function(k) { reviewLoadRows(k) })
    })
  }

  // The next page of groups, appended, when the queue holds more than are loaded.
  function reviewMore() {
    if (reviewLoading || !Insight.reviewHasMore(reviewRows.length, reviewRemaining, reviewLastPage)) return
    var g = reviewGen
    reviewLoading = true
    var path = "/review?limit=" + Insight.REVIEW_PAGE + "&offset=" + Insight.reviewNextOffset(reviewRows)
    Backend.request("GET", path, function(xhr) {
      if (g !== reviewGen) return
      reviewLoading = false
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j || !j.rows) { toast(envelopeText(xhr)); return }
      reviewLastPage = j.rows.length
      reviewRemaining = Number(j.remaining) || 0
      reviewRows = reviewRows.concat(j.rows)
    })
  }

  // POST /smart/rows: the next page (50) of an expanded group's rows.
  function reviewLoadRows(k) {
    var grp = Insight.groupByKey(reviewRows, k)
    if (!grp || reviewRowsLoading[k]) return
    var have = (reviewRowsByKey[k] || []).length
    var ids = Insight.groupPageIds(grp, have)
    if (ids.length === 0) return
    var g = reviewGen
    var l = Object.assign({}, reviewRowsLoading)
    l[k] = true
    reviewRowsLoading = l
    Backend.json("POST", "/smart/rows", { ids: ids }, function(xhr) {
      if (g !== reviewGen) return
      reviewRowsLoading = withoutKey(reviewRowsLoading, k)
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j || !j.rows) { toast(envelopeText(xhr)); return }
      var m = Object.assign({}, reviewRowsByKey)
      m[k] = (reviewRowsByKey[k] || []).concat(j.rows)
      reviewRowsByKey = m
    })
  }

  // Enter: a group shows or hides its rows (the first page loads when shown); on "more" the next page loads.
  function reviewExpandCurrent() {
    var it = review.cur
    if (!it || it.kind === "row") return
    var k = it.gkey
    if (it.kind === "more") { reviewLoadRows(k); return }
    var e = Object.assign({}, reviewExpanded)
    if (e[k]) { delete e[k]; reviewExpanded = e; return }
    e[k] = true
    reviewExpanded = e
    if (!(reviewRowsByKey[k] && reviewRowsByKey[k].length)) reviewLoadRows(k)
  }

  // Space: a row flips; on the header, every row of the group flips (all ticked unless all are).
  function reviewTickCurrent() {
    var it = review.cur
    var grp = groupAt()
    if (!it || !grp || it.kind === "more") return
    if (it.kind === "row") { reviewUnticked = Insight.toggleTick(reviewUnticked, it.gkey, Number(it.row.id)); return }
    reviewUnticked = Insight.setGroupTicks(reviewUnticked, it.gkey, grp, !Insight.allTicked(grp, reviewUnticked))
  }

  // a (on) and n (off): every row of the group.
  function reviewSetGroupTicks(on) {
    var grp = groupAt()
    if (!grp) return
    reviewUnticked = Insight.setGroupTicks(reviewUnticked, Insight.gkey(grp), grp, on)
  }

  // 1 2 3: the group's pending category becomes suggestion n. Nothing is written until y.
  function reviewPick(n) {
    var grp = groupAt()
    if (!grp) return
    var opt = Insight.optionFor(grp.options, n)
    if (!opt) { toast("this group has no suggestion " + n); return }
    setPending(grp, Number(opt.category_id), String(opt.path))
  }

  function setPending(grp, catId, path) {
    var p = Object.assign({}, reviewPending)
    p[Insight.gkey(grp)] = { category_id: catId, path: path }
    reviewPending = p
    toast("pending: " + path + "   y applies to the ticked rows")
  }

  // c opens the normal picker; the category it returns is the group's pending category (see
  // reviewPendingPick). C: the Phase 4 preview of the group's representative row.
  function reviewCategorize(smart) {
    var grp = groupAt()
    if (!grp) return
    if (smart) {
      catIds = [Number(grp.id)]
      smartFlow = true
      reviewPickFlow = false
      openCategorize()
      return
    }
    catIds = Insight.groupIds(grp)
    smartFlow = false
    reviewPickFlow = true
    reviewPickKey = Insight.gkey(grp)
    openCategorize()
    picker.title = "Pending category for " + String(grp.payee)
  }

  // The picker chose catId (null: "Uncategorized", which is not a pending choice).
  function reviewPendingPick(catId) {
    var grp = Insight.groupByKey(reviewRows, reviewPickKey)
    reviewPickFlow = false
    if (catId === null) { toast("pick a category; uncategorized is not a pending choice"); return }
    if (!grp) { toast("that group left the queue"); return }
    setPending(grp, catId, Tree.pathOf(categoryList, catId) || String(catId))
  }

  // y: the ticked rows of the group become its pending category, in one undoable request. The group
  // leaves the queue at once; a reload then brings back whatever was not ticked.
  function reviewApply() {
    var grp = groupAt()
    if (!grp) return
    var k = Insight.gkey(grp)
    var plan = Insight.applyPlan(grp, Insight.pendingFor(reviewPending, grp, suggState !== null && suggState.enabled === true),
                                 reviewUnticked)
    if (!plan.ok) { toast(plan.message); return }
    if (reviewInFlight[k]) return
    var f = Object.assign({}, reviewInFlight)
    f[k] = true
    reviewInFlight = f
    Backend.json("POST", "/transactions/categorize", { ids: plan.ids, category_id: plan.category_id }, function(xhr) {
      reviewInFlight = withoutKey(reviewInFlight, k)
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { toast(envelopeText(xhr)); return }
      var left = plan.ids.length - (Number(j.changed) || 0)
      toast(String(j.label || plan.path) + (left > 0 ? "  (" + Insight.fmtN(left) + " not changed)" : "") + "   u to undo")
      if (!reviewOpen) { afterChange(); return }
      reviewDirty = true
      reviewRows = Insight.removeGroup(reviewRows, k)
      reviewRemaining = Math.max(0, reviewRemaining - 1)
      reviewPending = withoutKey(reviewPending, k)
      reviewUnticked = withoutKey(reviewUnticked, k)
      reviewExpanded = withoutKey(reviewExpanded, k)
      reviewRowsByKey = withoutKey(reviewRowsByKey, k)
      reviewRowsLoading = withoutKey(reviewRowsLoading, k)
      reviewReload(false)
    })
  }

  function reviewAct(id) {
    if (id === "review.close") {
      if (Insight.anyExpanded(reviewExpanded)) { reviewExpanded = ({}); return }
      closeReview()
    }
    else if (id === "review.more") reviewMore()
    else if (id === "review.apply") reviewApply()
    else if (id === "review.tick") reviewTickCurrent()
    else if (id === "review.all") reviewSetGroupTicks(true)
    else if (id === "review.none") reviewSetGroupTicks(false)
    else if (id === "review.expand") reviewExpandCurrent()
    else if (id === "review.categorize") reviewCategorize(false)
    else if (id === "review.smart") reviewCategorize(true)
    else if (id === "review.undo") history("undo")
    else if (id.indexOf("review.pick") === 0) reviewPick(Number(id.substring(11)))
  }

  // ---- detail pane suggestion (uncategorized rows only)
  function loadDetailSuggestion() {
    var t = detailData
    if (!t) return
    var uncat = t.category_id === undefined ? !t.category : t.category_id === null
    if (!uncat) { detailSuggGen++; detailSugg = null; return }
    var id = Number(t.id)
    var g = ++detailSuggGen
    Backend.json("POST", "/suggest", { ids: [id] }, function(xhr) {
      if (g !== detailSuggGen) return
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { detailSugg = null; suggError = envelopeText(xhr); return }
      suggError = ""
      suggState = { enabled: j.enabled === true, reason: j.reason || null, accuracy: j.accuracy || null }
      var row = null
      var list = j.suggestions || []
      for (var i = 0; i < list.length; i++) if (Number(list[i].id) === id) row = list[i]
      detailSugg = row
    })
  }

  // Ctrl-y in the detail pane: accept the top suggestion (undoable with u).
  function suggestAccept() {
    var t = detailData
    if (!t) return
    if (!suggState || suggState.enabled !== true) { toast("suggestions are off: " + Insight.offReason(suggState)); return }
    var top = Insight.topOption(detailSugg)
    if (!top) { toast("no suggestion for this row"); return }
    var id = Number(t.id)
    Backend.json("POST", "/transactions/categorize", { ids: [id], category_id: Number(top.category_id) }, function(xhr) {
      var j = xhr.status === 200 ? parseJson(xhr.responseText) : null
      if (!j) { toast(envelopeText(xhr)); return }
      toast(String(j.label || top.path) + "   u to undo")
      afterChange()
      loadDetail()
    })
  }

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
      case "sort.notes": applySort("notes"); break
      case "notes.edit": startNotes(); break
      case "sidebar.toggle":
        sidebarOpen = !sidebarOpen
        if (!sidebarOpen && zone === "sidebar") zone = "table"
        break
      case "dash.open": openDashboard(); break
      case "review.open": openReview(); break
      case "reports.open": openReports(); break
      case "suggest.accept": suggestAccept(); break
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
      case "mark.toggle": toggleMarkCursor(); break
      case "visual.toggle": toggleVisual(); break
      case "mark.all": markAll(); break
      case "marks.clear": escapeTable(); break
      case "categorize": startCategorize(false); break
      case "categorizeSmart": startCategorize(true); break
      case "manager.open": openManager(); break
      case "undo": history("undo"); break
      case "redo": history("redo"); break
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
    if (visualOn) refreshMarks()
    if (detailOpen) {
      detailData = null
      detailTimer.restart()
    }
  }

  onSidebarOpenChanged: if (!sidebarOpen && zone === "sidebar") zone = "table"
  onManagerOpenChanged: if (!managerOpen) Qt.callLater(root.restoreFocus)
  onCategoryListChanged: mgrSync()

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
        visible: root.markCount > 0
        text: (root.visualOn ? "VISUAL   " : "") + Query.fmtCount(root.markCount) + " marked"
        color: Commons.Color.accent
        font.family: Style.font.family
        font.pixelSize: Style.font.body
        font.bold: true
      }
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
        suggState: root.suggState
        suggRow: root.detailSugg
        suggError: root.suggError
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
        text: "j/k move   Space mark   V range   C-a all   c categorize   m categories   u undo   Tab pane   / search   f filter   ? help   q quit"
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

  NotesDialog {
    id: notesDlg
    anchors.fill: parent
    onSubmitted: function(text) { root.saveNotes(text) }
    onCancelled: root.noteGen++
    onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
  }

  HelpOverlay {
    id: help
    anchors.fill: parent
    onClosed: Qt.callLater(root.restoreFocus)
  }

  CategoryManager {
    id: manager
    anchors.fill: parent
    open: root.managerOpen
    rows: root.categoryList
    cursorIndex: root.mgrIndex
    maxDepth: root.maxDepth
    status: root.mgrStatus
    statusError: root.mgrStatusError
    onAct: function(id) { root.mgrAct(id) }
  }

  Dashboard {
    id: dashView
    anchors.fill: parent
    open: root.dashOpen
    tab: root.dashTab
    dash: root.dashData
    dashError: root.dashError
    includeTransfers: root.dashTransfers
    insight: root.insightData
    insightErr: root.insightErr
    cursors: root.dashCursors
    onAct: function(id) { root.dashAct(id) }
    onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
  }

  ReviewQueue {
    id: review
    anchors.fill: parent
    open: root.reviewOpen
    groups: root.reviewRows
    mode: root.reviewMode
    remaining: root.reviewRemaining
    startRemaining: root.reviewStart
    loading: root.reviewLoading
    error: root.reviewError
    suggState: root.suggState
    suggError: root.suggError
    pending: root.reviewPending
    unticked: root.reviewUnticked
    expanded: root.reviewExpanded
    rowsByKey: root.reviewRowsByKey
    loadingByKey: root.reviewRowsLoading
    onAct: function(id) { root.reviewAct(id) }
    onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
  }

  Reports {
    id: reportsView
    anchors.fill: parent
    open: root.reportsOpen
    onAskText: function(ctx, title, hint, placeholder, initial) { root.openReportPrompt(ctx, title, hint, placeholder, initial) }
    onCloseRequested: root.reportsOpen = false
    onOpenChanged: if (!open) { root.reportsOpen = false; Qt.callLater(root.restoreFocus) }
  }

  SmartPreview {
    id: smart
    anchors.fill: parent
    open: root.smartOpen
    clusters: root.smartClusters
    summary: root.smartSummary
    selected: root.smartSelected
    expanded: root.smartExpanded
    rowsByKey: root.smartRows
    loadingByKey: root.smartLoadingByKey
    threshold: root.smartThreshold
    overwrite: root.smartOverwrite
    makeRule: root.smartRule
    busy: root.smartBusy
    error: root.smartError
    categoryPath: root.smartCat ? String(root.smartCat.path) : ""
    onAct: function(id) { root.smartAct(id) }
    onOpenChanged: if (!open) Qt.callLater(root.restoreFocus)
  }


}
