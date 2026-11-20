import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../keys/Keymap.js" as Keymap
import "../state/Insight.js" as Insight

// Dashboard (D): a full-pane overlay with four tabs. Dashboard = monthly bars (out vs in), category
// bars with share %, top payees, and the transfers-excluded note. Recurring, Anomalies and Transfers
// are lists; Enter jumps the table to the row's rows. AppShell owns every request and every action:
// this view renders what it is given and forwards its keys as act(id). Owns the keyboard while open.
FocusScope {
  id: root

  property bool open: false
  property int tab: 0                 // index into Insight.TABS
  property var dash: null             // GET /dashboard body, or null
  property string dashError: ""       // the backend's error envelope for /dashboard, or ""
  property bool includeTransfers: false
  property var insight: ({})          // tab name -> body (recurring: {series}, anomalies: {items}, transfers: {pairs,...}) or null
  property var insightErr: ({})       // tab name -> the backend's error envelope text, or ""
  property var cursors: ({})          // tab name -> row index of that list

  readonly property string tabName: Insight.TABS[tab]
  readonly property var curBody: insight[tabName] || null
  readonly property string curError: insightErr[tabName] || ""
  readonly property var items: tabName === "recurring" ? (curBody && curBody.series ? curBody.series : [])
                             : tabName === "anomalies" ? (curBody && curBody.items ? curBody.items : [])
                             : tabName === "transfers" ? (curBody && curBody.pairs ? curBody.pairs : [])
                             : []
  readonly property int cursor: Insight.clamp(cursors[tabName] || 0, items.length)
  readonly property string listHeader: tabName === "recurring" ? (curBody ? Insight.recurringHeader(items) : "")
                                     : tabName === "anomalies" ? (curBody ? Insight.anomaliesHeader(items) : "")
                                     : tabName === "transfers" ? (curBody ? Insight.transfersHeader(curBody) : "")
                                     : ""
  readonly property real maxMonth: Math.max(1, Insight.maxOf(dash && dash.months ? dash.months : [], "out"),
                                             Insight.maxOf(dash && dash.months ? dash.months : [], "in"))
  readonly property var catRows: dash && dash.by_category ? Insight.categoryBars(dash.by_category, dash.totals ? dash.totals.out : 0, Style.space(260)) : []

  readonly property color warnColor: Commons.Color.urgent
  readonly property int barMax: Style.space(260)

  signal act(string id)

  visible: open
  z: 18

  // Takes the keyboard (AppShell calls this when the overlay opens or an overlay above it closes).
  function focusKeys() { keyCatcher.forceActiveFocus() }

  function handle(id) {
    // On the dashboard tab j/k scroll the page; the lists move their cursor (AppShell keeps it).
    if (tab === 0 && (id === "dash.down" || id === "dash.up")) {
      var max = Math.max(0, flick.contentHeight - flick.height)
      flick.contentY = Math.max(0, Math.min(max, flick.contentY + (id === "dash.down" ? 40 : -40)))
      return
    }
    act(id)
  }

  // Swallows clicks so the table underneath does not react.
  MouseArea { anchors.fill: parent }

  Rectangle {
    anchors.fill: parent
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: Commons.Color.menu.border
    radius: Style.cornerRadius
  }

  // Keys live on this item; the children do not take focus.
  Item {
    id: keyCatcher
    objectName: "dashboardKeys"
    anchors.fill: parent
    focus: true
    Keys.onPressed: function(e) {
      var id = Keymap.resolve(["dash"], Keymap.token(e))
      if (id === "") return
      e.accepted = true
      root.handle(id)
    }
  }

  ColumnLayout {
    anchors.fill: parent
    anchors.margins: Style.spacing.xxl
    spacing: Style.spacing.md

    // Tab strip: the active tab is accented.
    RowLayout {
      Layout.fillWidth: true
      spacing: Style.spacing.lg * 2

      Repeater {
        model: Insight.TABS
        delegate: Text {
          required property string modelData
          required property int index
          text: Insight.tabTitle(index)
          color: index === root.tab ? Commons.Color.accent : Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.subtitle
          font.bold: index === root.tab
        }
      }
      Item { Layout.fillWidth: true }
      Text {
        text: root.tab === 0 && root.dash ? Insight.rangeText(root.dash) : ""
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    Text {
      Layout.fillWidth: true
      visible: root.dashError !== "" && root.tab === 0
      text: root.dashError
      color: root.warnColor
      font.family: Style.font.family
      font.pixelSize: Style.font.body
      wrapMode: Text.WordWrap
    }

    // ---- Dashboard tab
    Flickable {
      id: flick
      visible: root.tab === 0
      Layout.fillWidth: true
      Layout.fillHeight: true
      clip: true
      contentWidth: width
      contentHeight: dashBody.implicitHeight
      boundsBehavior: Flickable.StopAtBounds

      ColumnLayout {
        id: dashBody
        width: flick.width
        spacing: Style.spacing.md

        Text {
          Layout.fillWidth: true
          visible: root.dash === null && root.dashError === ""
          text: "loading..."
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.body
        }

        Text {
          Layout.fillWidth: true
          visible: root.dash !== null
          text: root.dash ? "Out " + Insight.moneyCents(root.dash.totals.out) + "   In " + Insight.moneyCents(root.dash.totals.in)
                            + "   Net " + Insight.signedMoneyCents(root.dash.totals.net) : ""
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.subtitle
          font.bold: true
        }

        Text {
          Layout.fillWidth: true
          visible: root.dash !== null
          text: root.dash ? Insight.transfersNote(root.dash.transfers_excluded, root.includeTransfers) : ""
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        // Monthly bars: out (accent) and in (muted) per month, oldest first.
        Text {
          visible: root.dash !== null
          text: "MONTHLY"
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          font.bold: true
        }
        Repeater {
          model: root.dash ? root.dash.months : []
          delegate: RowLayout {
            required property var modelData
            Layout.fillWidth: true
            spacing: Style.spacing.md
            Text {
              Layout.preferredWidth: Style.space(70)
              text: Insight.monthLabel(modelData.month)
              color: Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
            }
            Column {
              spacing: Style.spacing.xxs
              Rectangle {
                height: Style.space(8)
                width: Insight.barWidth(modelData.out, root.maxMonth, root.barMax)
                color: Commons.Color.accent
                visible: width > 0
              }
              Rectangle {
                height: Style.space(8)
                width: Insight.barWidth(modelData.in, root.maxMonth, root.barMax)
                color: Commons.Color.muted
                visible: width > 0
              }
            }
            Text {
              Layout.fillWidth: true
              text: "out " + Insight.moneyCents(modelData.out) + "   in " + Insight.moneyCents(modelData.in)
                    + "   net " + Insight.signedMoneyCents(modelData.net)
              color: Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
          }
        }

        // Category bars with the share of total out.
        Text {
          visible: root.dash !== null
          text: "CATEGORIES (OUT)"
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          font.bold: true
        }
        Repeater {
          model: root.catRows
          delegate: ColumnLayout {
            required property var modelData
            Layout.fillWidth: true
            spacing: Style.spacing.xxs
            Text {
              Layout.fillWidth: true
              text: modelData.path
              color: Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.spacing.md
              Rectangle {
                height: Style.space(8)
                width: modelData.width
                color: Commons.Color.accent
                visible: width > 0
              }
              Text {
                text: modelData.label
                color: Commons.Color.muted
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
              }
            }
          }
        }

        // Top payees, by normalized payee.
        Text {
          visible: root.dash !== null
          text: "TOP PAYEES (OUT)"
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          font.bold: true
        }
        Repeater {
          model: root.dash ? root.dash.top_payees : []
          delegate: Text {
            required property var modelData
            Layout.fillWidth: true
            text: modelData.payee + "   " + Insight.moneyCents(modelData.out) + "   " + modelData.count + " txns"
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
            elide: Text.ElideRight
          }
        }
      }
    }

    // ---- Recurring, Anomalies, Transfers tabs
    Text {
      Layout.fillWidth: true
      visible: root.tab !== 0
      text: root.curError !== "" ? root.curError
          : (root.curBody === null ? "loading..." : root.listHeader)
      color: root.curError !== "" ? root.warnColor : Commons.Color.menu.text
      font.family: Style.font.family
      font.pixelSize: Style.font.body
      font.bold: true
      wrapMode: Text.WordWrap
    }

    ListView {
      id: list
      visible: root.tab !== 0
      Layout.fillWidth: true
      Layout.fillHeight: true
      clip: true
      model: root.tab !== 0 ? root.items : []
      currentIndex: root.cursor
      boundsBehavior: Flickable.StopAtBounds
      onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)

      delegate: Rectangle {
        id: line
        required property var modelData
        required property int index
        readonly property bool current: index === root.cursor
        width: list.width
        height: Style.space(40)
        color: current ? Style.selectedAccentFill : "transparent"

        ColumnLayout {
          anchors.fill: parent
          anchors.leftMargin: Style.spacing.md
          anchors.rightMargin: Style.spacing.md
          spacing: Style.spacing.xxs
          Text {
            Layout.fillWidth: true
            elide: Text.ElideRight
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
            font.bold: true
            text: root.tabName === "recurring" ? String(modelData.payee) + (modelData.active ? "" : "   (lapsed)")
                : root.tabName === "anomalies" ? Insight.anomalyLine(modelData)
                : Insight.pairLine(modelData)
          }
          Text {
            Layout.fillWidth: true
            elide: Text.ElideRight
            color: Commons.Color.muted
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            text: root.tabName === "recurring" ? Insight.seriesLine(modelData)
                : root.tabName === "anomalies" ? String(modelData.detail || "")
                : "out #" + modelData.out_id + "  \u00b7  in #" + modelData.in_id
          }
        }
      }
    }

    Text {
      visible: root.tab !== 0 && root.items.length === 0 && root.curError === "" && root.curBody !== null
      Layout.fillWidth: true
      text: root.tabName === "recurring" ? "No recurring series." : root.tabName === "anomalies" ? "No anomalies." : "No transfer pairs."
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
    }

    Text {
      Layout.fillWidth: true
      text: root.tab === 0
          ? "Tab / S-Tab tabs   j/k scroll   t transfers in or out   Esc close"
          : "Tab / S-Tab tabs   j/k move   Enter jump the table to these rows   Esc close"
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      elide: Text.ElideRight
    }
  }
}
