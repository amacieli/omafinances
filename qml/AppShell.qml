import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "state"

// OmaFinances window content (Phase 0): real backend status plus the most recent transactions.
// Read-only. Keybindings are documented in docs/KEYS.md, which is the source of truth; keep the
// key table in onKeyPressed below in sync with it.
FocusScope {
  id: root

  signal closeRequested()

  // ------------------------------------------------------------------ tuning
  readonly property int pageSize: 200
  // The next page is requested when the selection or the view gets this close to the loaded end.
  readonly property int prefetchRows: 40

  // ------------------------------------------------------------------ state
  // "loading" | "ok" | "unreachable" | "error" (an HTTP failure such as 401)
  property string backendState: "loading"
  property string backendDetail: ""
  property var serverStatus: null   // parsed /status body
  property int total: 0             // rows the backend reports for /transactions
  property bool pageBusy: false
  property bool seekEnd: false      // G was pressed: keep loading until the last row is in
  property int generation: 0        // bumped by reload(); responses from older generations are dropped
  // Settings -> Appearance scale factor. shell.qml scales the whole UI by it.
  property real uiScale: 1.0

  readonly property bool dbError: serverStatus !== null && serverStatus.db !== undefined
                                  && serverStatus.db.state === "error"
  readonly property int rowHeight: Style.font.body + Style.spacing.xl * 2
  readonly property int padX: Style.spacing.rowPaddingX
  readonly property int gap: Style.spacing.lg
  // Extra air between the amount (right-aligned) and the account column.
  readonly property int gapWide: Style.spacing.xxl

  // Monospace column widths, derived from the real advance width of the font.
  TextMetrics {
    id: glyph
    font.family: Style.font.family
    font.pixelSize: Style.font.body
    text: "0000000000"
  }
  readonly property real cw: glyph.advanceWidth / 10
  readonly property real colDate: cw * 10
  readonly property real colAmount: cw * 13
  readonly property real colAccount: cw * 16
  readonly property real colCategory: cw * 24

  // ------------------------------------------------------------------ helpers
  function parseJson(text) {
    try { return JSON.parse(text) } catch (e) { return null }
  }

  function describeHttp(xhr) {
    if (xhr.status === 401) return "backend rejected the session token (401)"
    return "HTTP " + xhr.status
  }

  function setUnreachable() {
    backendState = "unreachable"
    backendDetail = "port " + Backend.port
  }

  // Amount sign convention (this DB): a positive amount is money OUT (spending) and is shown as
  // plain text. A negative amount is money IN (income, refunds) and is shown in the accent color.
  // Non-USD currencies get their code appended; USD is the default and stays unmarked.
  function formatAmount(a, currency) {
    var n = Number(a)
    if (!isFinite(n)) return ""
    var parts = Math.abs(n).toFixed(2).split(".")
    var whole = parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ",")
    var s = (n < 0 ? "-" : "") + whole + "." + parts[1]
    if (currency && currency !== "USD") s += " " + currency
    return s
  }

  function statusLine() {
    if (backendState === "loading") return "connecting..."
    if (backendState === "unreachable") return "Backend not responding (" + backendDetail + ")"
    if (backendState === "error") return "Backend error: " + backendDetail
    if (serverStatus === null) return ""
    var s = serverStatus
    var db = s.db || {}
    var parts = ["v" + s.version,
                 (db.transactions || 0) + " txns",
                 (db.accounts || 0) + " accounts",
                 (db.categories || 0) + " categories"]
    if (db.last_txn_date) parts.push("last txn " + db.last_txn_date)
    if (db.last_sync) parts.push("synced " + db.last_sync)
    parts.push("mcp " + (s.mcp && s.mcp.enabled ? "on" : "off"))
    return parts.join("  \u00b7  ")
  }

  function bodyMessage() {
    if (rows.count > 0) return ""
    if (backendState === "unreachable") return "Backend not responding"
    if (backendState === "error") return "Backend error: " + backendDetail
    if (backendState === "loading") return "Loading..."
    if (dbError) return "No transactions available: the database is locked."
    return "No transactions yet."
  }

  // ------------------------------------------------------------------ backend
  ListModel { id: rows }

  function loadSettings() {
    Backend.request("GET", "/settings", function(xhr) {
      if (xhr.status !== 200) return
      var j = parseJson(xhr.responseText)
      if (j && typeof j.scale_factor === "number" && j.scale_factor > 0)
        root.uiScale = Math.min(3, Math.max(0.5, j.scale_factor))
    })
  }

  function loadStatus() {
    var gen = generation
    Backend.request("GET", "/status", function(xhr) {
      if (gen !== root.generation) return
      if (xhr.status === 0) { setUnreachable(); return }
      var j = parseJson(xhr.responseText)
      if (xhr.status !== 200 || !j) {
        backendState = "error"
        backendDetail = describeHttp(xhr)
        return
      }
      serverStatus = j
      backendState = "ok"
    })
  }

  // Appends the next page. Pages are keyed by the number of rows already held, so a page is
  // never requested twice and a reload cannot splice old rows into the new list.
  function loadPage() {
    if (pageBusy) return
    if (rows.count > 0 && rows.count >= total) { seekFinish(); return }
    pageBusy = true
    var gen = generation
    var offset = rows.count
    Backend.request("GET", "/transactions?limit=" + pageSize + "&offset=" + offset, function(xhr) {
      if (gen !== root.generation) return
      pageBusy = false
      if (xhr.status === 0) { setUnreachable(); seekEnd = false; return }
      var j = parseJson(xhr.responseText)
      if (xhr.status !== 200 || !j || !j.rows) {
        backendState = "error"
        backendDetail = describeHttp(xhr)
        seekEnd = false
        return
      }
      for (var i = 0; i < j.rows.length; i++) {
        var r = j.rows[i]
        rows.append({
          date: String(r.date || "").substring(0, 10),
          payee: String(r.payee || ""),
          amount: Number(r.amount) || 0,
          currency: String(r.currency || ""),
          account: String(r.account || ""),
          category: r.category ? String(r.category) : ""
        })
      }
      total = Number(j.total) || rows.count
      // An empty page with a larger total would otherwise keep asking forever.
      if (j.rows.length === 0) total = rows.count
      backendState = "ok"
      if (seekEnd) seekFinish()
      else maybePrefetch()
    })
  }

  function reload() {
    generation++
    rows.clear()
    total = 0
    pageBusy = false
    seekEnd = false
    serverStatus = null
    backendState = "loading"
    backendDetail = ""
    list.currentIndex = 0
    loadStatus()
    loadPage()
  }

  // ------------------------------------------------------------------ navigation
  function select(i) {
    if (rows.count === 0) return
    list.currentIndex = Math.max(0, Math.min(rows.count - 1, i))
    list.positionViewAtIndex(list.currentIndex, ListView.Contain)
    maybePrefetch()
  }

  function moveBy(n) {
    select(list.currentIndex + n)
  }

  function pageStep() {
    return Math.max(1, Math.floor(list.height / rowHeight / 2))
  }

  function maybePrefetch() {
    if (rows.count < total && list.currentIndex >= rows.count - prefetchRows) loadPage()
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

  // ------------------------------------------------------------------ keys
  // Text fields would own keys while focused; none exist in Phase 0.
  focus: true
  Keys.onPressed: function(event) {
    var ctrl = (event.modifiers & Qt.ControlModifier) !== 0
    var t = event.text
    var k = event.key
    var handled = true
    if (k === Qt.Key_Escape || (!ctrl && t === "q")) root.closeRequested()
    else if (!ctrl && (t === "j" || k === Qt.Key_Down)) root.moveBy(1)
    else if (!ctrl && (t === "k" || k === Qt.Key_Up)) root.moveBy(-1)
    else if (!ctrl && t === "g") root.select(0)
    else if (!ctrl && t === "G") root.goEnd()
    else if (ctrl && k === Qt.Key_D) root.moveBy(root.pageStep())
    else if (ctrl && k === Qt.Key_U) root.moveBy(-root.pageStep())
    else if (!ctrl && t === "r") root.reload()
    else handled = false
    event.accepted = handled
  }

  // ------------------------------------------------------------------ layout
  ColumnLayout {
    anchors.fill: parent
    anchors.margins: Style.spacing.xxl
    spacing: 0

    // Header: app name and the backend status line.
    RowLayout {
      Layout.fillWidth: true
      spacing: root.gap * 2

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
      id: banner
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

    // Column titles, aligned with the row cells below.
    RowLayout {
      Layout.fillWidth: true
      Layout.topMargin: Style.spacing.xl
      Layout.leftMargin: root.padX
      Layout.rightMargin: root.padX
      spacing: root.gap
      height: root.rowHeight

      Text { Layout.preferredWidth: root.colDate; Layout.maximumWidth: root.colDate; text: "DATE"; color: Commons.Color.muted; font.family: Style.font.family; font.pixelSize: Style.font.caption }
      Text { Layout.fillWidth: true; text: "PAYEE"; color: Commons.Color.muted; font.family: Style.font.family; font.pixelSize: Style.font.caption }
      Text { Layout.preferredWidth: root.colAmount; Layout.maximumWidth: root.colAmount; horizontalAlignment: Text.AlignRight; text: "AMOUNT"; color: Commons.Color.muted; font.family: Style.font.family; font.pixelSize: Style.font.caption }
      Text { Layout.preferredWidth: root.colAccount; Layout.maximumWidth: root.colAccount; Layout.leftMargin: root.gapWide; text: "ACCOUNT"; color: Commons.Color.muted; font.family: Style.font.family; font.pixelSize: Style.font.caption }
      Text { Layout.preferredWidth: root.colCategory; Layout.maximumWidth: root.colCategory; text: "CATEGORY"; color: Commons.Color.muted; font.family: Style.font.family; font.pixelSize: Style.font.caption }
    }

    Rectangle {
      Layout.fillWidth: true
      Layout.leftMargin: root.padX
      Layout.rightMargin: root.padX
      implicitHeight: Style.space(1)
      color: Commons.Color.menu.border
    }

    // Body: the transaction table, or the empty / unreachable message.
    Item {
      Layout.fillWidth: true
      Layout.fillHeight: true

      ListView {
        id: list
        anchors.fill: parent
        model: rows
        clip: true
        currentIndex: 0
        keyNavigationEnabled: false
        boundsBehavior: Flickable.StopAtBounds
        // Scrolling near the end pulls the next page, same as moving the selection there.
        onContentYChanged: {
          if (contentY + height > contentHeight - root.rowHeight * root.prefetchRows) root.loadPage()
        }
        delegate: Rectangle {
          id: rowItem
          required property int index
          required property string date
          required property string payee
          required property real amount
          required property string currency
          required property string account
          required property string category

          readonly property bool selected: index === list.currentIndex
          width: list.width
          height: root.rowHeight
          color: selected ? Commons.Color.menu.selectedBackground
               : (index % 2 === 1 ? Qt.alpha(Commons.Color.menu.text, 0.04) : "transparent")

          MouseArea {
            anchors.fill: parent
            onClicked: root.select(rowItem.index)
          }

          RowLayout {
            anchors.fill: parent
            anchors.leftMargin: root.padX
            anchors.rightMargin: root.padX
            spacing: root.gap

            Text {
              Layout.preferredWidth: root.colDate
              Layout.maximumWidth: root.colDate
              text: rowItem.date
              color: rowItem.selected ? Commons.Color.menu.selectedText : Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              elide: Text.ElideRight
            }
            Text {
              Layout.fillWidth: true
              Layout.minimumWidth: 0
              text: rowItem.payee
              color: rowItem.selected ? Commons.Color.menu.selectedText : Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              elide: Text.ElideRight
            }
            Text {
              Layout.preferredWidth: root.colAmount
              Layout.maximumWidth: root.colAmount
              horizontalAlignment: Text.AlignRight
              text: root.formatAmount(rowItem.amount, rowItem.currency)
              // Money in (negative) stands out in the accent color; spending stays plain text.
              color: rowItem.amount < 0 ? Commons.Color.accent
                   : (rowItem.selected ? Commons.Color.menu.selectedText : Commons.Color.menu.text)
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              font.bold: rowItem.amount < 0
              elide: Text.ElideRight
            }
            Text {
              Layout.preferredWidth: root.colAccount
              Layout.maximumWidth: root.colAccount
              Layout.leftMargin: root.gapWide
              text: rowItem.account
              color: rowItem.selected ? Commons.Color.menu.selectedText : Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              elide: Text.ElideRight
            }
            Text {
              Layout.preferredWidth: root.colCategory
              Layout.maximumWidth: root.colCategory
              text: rowItem.category !== "" ? rowItem.category : "\u2014"
              color: rowItem.selected ? Commons.Color.menu.selectedText : Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              elide: Text.ElideRight
            }
          }
        }
      }

      // Thin position indicator at the right edge; a sibling of the list so it does not scroll.
      Rectangle {
        visible: list.contentHeight > list.height
        x: list.width - width - Style.spacing.xxs
        width: Style.space(3)
        radius: width / 2
        height: Math.max(Style.space(24), list.height * list.height / list.contentHeight)
        y: (list.height - height) * list.visibleArea.yPosition / Math.max(0.0001, 1 - list.visibleArea.heightRatio)
        color: Qt.alpha(Commons.Color.menu.text, 0.35)
      }

      Text {
        anchors.centerIn: parent
        visible: rows.count === 0
        text: root.bodyMessage()
        color: root.backendState === "unreachable" || root.backendState === "error"
               ? Commons.Color.urgent : Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
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
      spacing: root.gap * 2

      Text {
        Layout.fillWidth: true
        elide: Text.ElideRight
        text: "j/k move   g/G ends   C-d/C-u page   r reload   q quit"
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
      Text {
        text: (rows.count > 0 ? (list.currentIndex + 1) : 0) + " / " + root.total
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }
  }
}
