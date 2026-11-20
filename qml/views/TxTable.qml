import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../state/Query.js" as Query

// The transaction table: column titles (click one to sort), banded rows, paging trigger.
// Rows come from AppShell's `rows` model. Keys and state live in AppShell; a click reports the row.
Item {
  id: root

  property var rowsModel: null
  property int currentIndex: 0
  property bool focusedPane: true
  property string sortKey: "date"
  property string sortDir: "desc"
  property int prefetchRows: 40

  signal rowClicked(int index)
  signal columnClicked(string key)
  signal nearEnd()

  readonly property alias listView: list

  // The highlighted row is set from outside (AppShell's cursor); the ListView only follows it.
  onCurrentIndexChanged: {
    if (list.currentIndex !== currentIndex) list.currentIndex = currentIndex
    list.positionViewAtIndex(currentIndex, ListView.Contain)
  }
  readonly property int rowHeight: Style.font.body + Style.spacing.xl * 2
  readonly property int padX: Style.spacing.rowPaddingX
  readonly property int gap: Style.spacing.lg
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

  function arrowFor(key) {
    return key === sortKey ? (sortDir === "asc" ? "  \u2191" : "  \u2193") : ""
  }

  // Column titles, aligned with the row cells below.
  RowLayout {
    id: head
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.top: parent.top
    anchors.leftMargin: root.padX
    anchors.rightMargin: root.padX
    spacing: root.gap
    height: root.rowHeight

    component ColTitle: Text {
      property string key: ""
      property string label: ""
      text: label + root.arrowFor(key)
      color: key === root.sortKey ? Commons.Color.accent : Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      font.bold: key === root.sortKey
      MouseArea { anchors.fill: parent; onClicked: root.columnClicked(parent.key) }
    }

    ColTitle { Layout.preferredWidth: root.colDate; Layout.maximumWidth: root.colDate; key: "date"; label: "DATE" }
    ColTitle { Layout.fillWidth: true; key: "payee"; label: "PAYEE" }
    ColTitle { Layout.preferredWidth: root.colAmount; Layout.maximumWidth: root.colAmount; horizontalAlignment: Text.AlignRight; key: "amount"; label: "AMOUNT" }
    ColTitle { Layout.preferredWidth: root.colAccount; Layout.maximumWidth: root.colAccount; Layout.leftMargin: root.gapWide; key: "account"; label: "ACCOUNT" }
    ColTitle { Layout.preferredWidth: root.colCategory; Layout.maximumWidth: root.colCategory; key: "category"; label: "CATEGORY" }
  }

  Rectangle {
    id: rule
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.top: head.bottom
    anchors.leftMargin: root.padX
    anchors.rightMargin: root.padX
    implicitHeight: Style.space(1)
    color: Commons.Color.menu.border
  }

  ListView {
    id: list
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.top: rule.bottom
    anchors.bottom: parent.bottom
    anchors.topMargin: Style.spacing.xs
    model: root.rowsModel
    clip: true
    keyNavigationEnabled: false
    boundsBehavior: Flickable.StopAtBounds
    // Scrolling near the end asks for the next page, same as moving the selection there.
    onContentYChanged: {
      if (contentY + height > contentHeight - root.rowHeight * root.prefetchRows) root.nearEnd()
    }

    delegate: Rectangle {
      id: rowItem
      required property int index
      required property var txid
      required property string date
      required property string payee
      required property real amount
      required property string currency
      required property string account
      required property string category
      required property bool pending
      required property bool recurring

      readonly property bool selected: index === root.currentIndex && root.focusedPane
      readonly property bool current: index === root.currentIndex
      width: list.width
      height: root.rowHeight
      color: current ? Style.selectedAccentFill
           : (index % 2 === 1 ? Qt.alpha(Commons.Color.menu.text, 0.04) : "transparent")

      MouseArea {
        anchors.fill: parent
        onClicked: root.rowClicked(rowItem.index)
      }

      RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.padX
        anchors.rightMargin: root.padX
        spacing: root.gap
        opacity: rowItem.pending ? 0.6 : 1

        Text {
          Layout.preferredWidth: root.colDate
          Layout.maximumWidth: root.colDate
          text: rowItem.date
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.body
          font.italic: rowItem.pending
          elide: Text.ElideRight
        }
        RowLayout {
          Layout.fillWidth: true
          Layout.minimumWidth: 0
          clip: true   // a long payee plus the pending tag must not run into the amount column
          spacing: Style.spacing.sm
          Text {
            visible: rowItem.recurring
            text: "\u21bb"
            color: Commons.Color.accent
            font.family: Style.font.family
            font.pixelSize: Style.font.body
          }
          Text {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: rowItem.payee
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.body
            font.italic: rowItem.pending
            elide: Text.ElideRight
          }
          Text {
            visible: rowItem.pending
            text: "pending"
            color: Commons.Color.muted
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            font.italic: true
          }
        }
        Text {
          Layout.preferredWidth: root.colAmount
          Layout.maximumWidth: root.colAmount
          horizontalAlignment: Text.AlignRight
          text: Query.fmtAmountRow(rowItem.amount, rowItem.currency)
          // Money in (negative) stands out in the accent color; spending stays plain text.
          color: rowItem.amount < 0 ? Commons.Color.accent : Commons.Color.menu.text
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
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.body
          elide: Text.ElideRight
        }
        // Category as a chip-like label; uncategorized is a muted dash.
        Item {
          Layout.preferredWidth: root.colCategory
          Layout.maximumWidth: root.colCategory
          Layout.fillHeight: true
          Rectangle {
            visible: rowItem.category !== ""
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(parent.width, catText.implicitWidth + Style.spacing.md * 2)
            height: catText.implicitHeight + Style.spacing.xxs * 2
            radius: Style.cornerRadius
            color: Qt.alpha(Commons.Color.accent, 0.12)
            Text {
              id: catText
              anchors.left: parent.left
              anchors.leftMargin: Style.spacing.md
              anchors.verticalCenter: parent.verticalCenter
              width: parent.width - Style.spacing.md * 2
              text: rowItem.category
              color: Commons.Color.accent
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
          }
          Text {
            visible: rowItem.category === ""
            anchors.verticalCenter: parent.verticalCenter
            text: "\u2014"
            color: Commons.Color.muted
            font.family: Style.font.family
            font.pixelSize: Style.font.body
          }
        }
      }
    }
  }

  // Thin position indicator at the right edge; a sibling of the list so it does not scroll.
  Rectangle {
    visible: list.contentHeight > list.height
    x: root.width - width - Style.spacing.xxs
    y: list.y + (list.height - height) * list.visibleArea.yPosition / Math.max(0.0001, 1 - list.visibleArea.heightRatio)
    width: Style.space(3)
    radius: width / 2
    height: Math.max(Style.space(24), list.height * list.visibleArea.heightRatio)
    color: Qt.alpha(Commons.Color.menu.text, 0.35)
  }
}
