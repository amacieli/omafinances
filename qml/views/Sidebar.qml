import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../common"
import "../state/Query.js" as Query

// Left sidebar: accounts, then the category tree with subtree counts, then Uncategorized.
// Rows come from AppShell's sideRows model: {rowKind: header|account|category|uncat, key, label,
// depth, count, sel}. The cursor and every key live in AppShell; a click reports the row.
Rectangle {
  id: root

  property var rowsModel: null
  property int cursor: 0
  property bool focusedPane: false

  signal rowClicked(int index)

  color: "transparent"
  border.width: Style.space(1)
  border.color: focusedPane ? Commons.Color.menu.selectedBorder : Commons.Color.menu.border
  radius: Style.cornerRadius
  clip: true

  readonly property int rowH: Style.font.body + Style.spacing.xl * 2
  readonly property int indentStep: Style.space(12)

  ListView {
    id: list
    anchors.fill: parent
    anchors.margins: Style.spacing.sm
    model: root.rowsModel
    clip: true
    interactive: false
    currentIndex: root.cursor
    boundsBehavior: Flickable.StopAtBounds
    onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)

    delegate: Item {
      id: row
      required property int index
      required property string rowKind
      required property string key
      required property string label
      required property int depth
      required property int count
      required property bool sel

      readonly property bool header: rowKind === "header"
      readonly property bool cursorHere: index === root.cursor && root.focusedPane && !header
      width: list.width
      height: header ? root.rowH + Style.spacing.md : root.rowH

      Rectangle {
        anchors.fill: parent
        anchors.topMargin: row.header ? Style.spacing.md : 0
        color: row.cursorHere ? Style.selectedAccentFill : "transparent"
        radius: Style.cornerRadius
      }

      // Facet marker: a thin accent bar on the left of the active account or category.
      Rectangle {
        visible: row.sel
        x: 0
        width: Style.space(3)
        height: row.height - (row.header ? Style.spacing.md : 0)
        y: row.header ? Style.spacing.md : 0
        color: Commons.Color.accent
      }

      MouseArea {
        anchors.fill: parent
        enabled: !row.header
        onClicked: root.rowClicked(row.index)
      }

      Text {
        visible: row.header
        anchors.left: parent.left
        anchors.leftMargin: Style.spacing.md
        anchors.verticalCenter: parent.verticalCenter
        text: row.label.toUpperCase()
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        font.bold: true
      }

      RowLayout {
        visible: !row.header
        anchors.fill: parent
        anchors.leftMargin: Style.spacing.md + row.depth * root.indentStep
        anchors.rightMargin: Style.spacing.md
        spacing: Style.spacing.sm

        Text {
          Layout.fillWidth: true
          Layout.minimumWidth: 0
          text: row.label
          color: row.sel ? Commons.Color.accent
               : (row.rowKind === "uncat" ? Commons.Color.muted : Commons.Color.menu.text)
          font.family: Style.font.family
          font.pixelSize: Style.font.body
          font.italic: row.rowKind === "uncat"
          font.bold: row.sel
          elide: Text.ElideRight
        }
        Text {
          text: Query.fmtCount(row.count)
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }
    }
  }
}
