import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../keys/Keymap.js" as Keymap
import "../state/Query.js" as Query

// The category manager (`m`): a full-pane overlay over the transaction window. It shows the tree
// from GET /categories (rows in tree order) with per-node counts, kind badges and depth guides.
// Keys come from the "manager" scope and are forwarded as act(id); AppShell owns the cursor,
// every action and every backend call. Owns the keyboard while open.
FocusScope {
  id: root

  property bool open: false
  property var rows: []            // /categories rows, tree order
  property int cursorIndex: 0      // highlighted row
  property int maxDepth: 0
  property string status: ""       // server messages and errors, shown above the hints
  property bool statusError: false

  readonly property int indentStep: Style.space(18)

  signal act(string id)

  visible: open
  z: 15

  // Takes the keyboard (AppShell calls this when the manager opens or an overlay above it closes).
  function focusKeys() { keys.forceActiveFocus() }

  function kindColor(kind) {
    if (kind === "income") return Commons.Color.accent
    if (kind === "transfer") return Commons.Color.muted
    return Commons.Color.urgent   // expense
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

  // Keys live on this item; the manager's own children do not take focus.
  Item {
    id: keys
    objectName: "categoryManagerKeys"
    anchors.fill: parent
    focus: true
    Keys.onPressed: function(e) {
      var id = Keymap.resolve(["manager"], Keymap.token(e))
      if (id === "") return
      e.accepted = true
      root.act(id)
    }
  }

  ColumnLayout {
    anchors.fill: parent
    anchors.margins: Style.spacing.xxl
    spacing: Style.spacing.md

    RowLayout {
      Layout.fillWidth: true
      spacing: Style.spacing.lg * 2

      Text {
        text: "Categories"
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }
      Text {
        Layout.fillWidth: true
        text: "counts: own transactions, then the whole subtree"
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        elide: Text.ElideRight
      }
      Text {
        text: "max depth " + root.maxDepth
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    Item {
      Layout.fillWidth: true
      Layout.fillHeight: true

      ListView {
        id: list
        anchors.fill: parent
        clip: true
        model: root.rows
        currentIndex: root.cursorIndex
        boundsBehavior: Flickable.StopAtBounds
        onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)

        delegate: Rectangle {
          id: line
          required property var modelData
          required property int index
          readonly property bool current: index === root.cursorIndex
          readonly property int depth: Number(modelData.depth) || 0
          readonly property int own: Number(modelData.count) || 0
          readonly property int subtree: Number(modelData.count_subtree) || 0
          width: list.width
          height: Style.spacing.popupRowHeight
          color: current ? Style.selectedAccentFill : "transparent"

          // One thin guide per ancestor level.
          Repeater {
            model: line.depth
            Rectangle {
              x: Style.spacing.md + index * root.indentStep + Math.round(root.indentStep / 2)
              width: Style.space(1)
              height: line.height
              color: Qt.alpha(Commons.Color.menu.text, 0.18)
            }
          }

          RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Style.spacing.md + line.depth * root.indentStep
            anchors.rightMargin: Style.spacing.md
            spacing: Style.spacing.md

            Text {
              Layout.fillWidth: true
              Layout.minimumWidth: 0
              text: String(line.modelData.name)
              color: line.current ? Commons.Color.menu.selectedText : Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              elide: Text.ElideRight
            }

            Rectangle {
              id: badge
              readonly property color tint: root.kindColor(line.modelData.kind)
              width: kindText.implicitWidth + Style.spacing.md * 2
              height: kindText.implicitHeight + Style.spacing.xxs * 2
              radius: Style.cornerRadius
              color: Qt.alpha(tint, 0.14)
              Text {
                id: kindText
                anchors.centerIn: parent
                text: String(line.modelData.kind || "expense")
                color: badge.tint
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
              }
            }

            Text {
              Layout.preferredWidth: Style.space(260)
              horizontalAlignment: Text.AlignRight
              text: Query.fmtCount(line.own) + " txns"
                    + (line.subtree !== line.own ? "   " + Query.fmtCount(line.subtree) + " in tree" : "")
              color: Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
          }
        }
      }

      Text {
        anchors.centerIn: parent
        visible: root.rows.length === 0
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        text: "No categories yet. Press a to add the first one, or Esc to close."
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
      }
    }

    Text {
      Layout.fillWidth: true
      visible: root.status !== ""
      text: root.status
      color: root.statusError ? Commons.Color.urgent : Commons.Color.accent
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      Layout.fillWidth: true
      text: "j/k move   a child   A sibling   r rename   H/L outdent/indent   J/K reorder   e kind   d delete   M merge   Enter filter   Esc close"
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      elide: Text.ElideRight
    }
  }
}
