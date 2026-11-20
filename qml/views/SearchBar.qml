import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "../keys/Keymap.js" as Keymap

// Slash search bar, docked at the top of the table pane. It is a view: AppShell owns the state
// (parse results, history, completions, commands) and sets the properties below; this file only
// draws them and reports keys (action) and clicks on a completion (completionPicked).
//
// Height is reserved while open (the hint row always exists), so opening the bar never moves
// the table's rows. The bar owns the keys it binds in the "search" scope while open; every other
// character is typed into the field.
Item {
  id: root

  property bool open: false
  property alias text: input.text          // changes come from the user or the owner
  property alias cursor: input.cursorPosition
  property alias field: input                // the TextField (focus is cleared by restoreFocus)
  property string hint: ""
  property bool hintError: false
  property var completions: []             // [{insert, label, detail}], empty = no list
  property int completionIndex: 0
  property int completionStart: 0          // UTF-16 start of the token being completed
  property int errorStart: -1              // UTF-16 span underlined in the field, -1 = none
  property int errorEnd: -1

  readonly property int maxRows: 8
  readonly property int hintHeight: Style.font.bodySmall + Style.spacing.xs * 2

  signal action(string id)
  signal userEdited()
  signal completionPicked(int index)

  visible: open
  height: open ? barCol.implicitHeight + Style.spacing.md * 2 : 0
  z: 5

  // Where a UTF-16 index sits inside the field (field-local x).
  function caretX(pos, t) {
    return input.positionToRectangle(Math.max(0, Math.min(pos, input.text.length))).x
  }

  function show(initial) {
    input.text = initial || ""
    open = true
    input.forceActiveFocus()
    input.cursorPosition = input.text.length
  }

  function hide() {
    if (!open) return
    open = false
  }

  Rectangle {
    anchors.fill: parent
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: Commons.Color.menu.selectedBorder
    radius: Style.cornerRadius
  }

  ColumnLayout {
    id: barCol
    anchors.fill: parent
    anchors.margins: Style.spacing.md
    spacing: Style.spacing.xs

    RowLayout {
      Layout.fillWidth: true
      spacing: Style.spacing.sm

      Text {
        text: "/"
        color: Commons.Color.accent
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }

      TextField {
        id: input
        Layout.fillWidth: true
        placeholderText: "payee, p:coffee, a:chase, d:last-month, sort:-amount, or :help"
        // The bar owns its keys while focused: this runs before the field's own key handling.
        Keys.priority: Keys.BeforeItem
        onTextEdited: root.userEdited()
        Keys.onPressed: function(e) {
          var id = Keymap.resolve(["search"], Keymap.token(e))
          if (id === "") return
          e.accepted = true
          root.action(id)
        }

          // Completion list under the token being completed (field-local, so it tracks the field), at most maxRows rows.
          Rectangle {
            id: popup
            visible: root.open && root.completions.length > 0
            readonly property real wanted: root.caretX(root.completionStart, input.text) - Style.spacing.sm
            x: Math.max(0, Math.min(wanted, input.width - width))
            // Sits under the hint row: field, column gap, hint row, column gap.
            y: input.height + Style.spacing.xs * 2 + root.hintHeight
            width: Math.min(input.width, Style.space(260))
            height: Math.min(root.completions.length, root.maxRows) * Style.spacing.popupRowHeight + Style.spacing.sm * 2
            z: 10
            color: Commons.Color.menu.background
            border.width: Style.space(1)
            border.color: Commons.Color.menu.border
            radius: Style.cornerRadius
            clip: true

            ListView {
              id: list
              anchors.fill: parent
              anchors.margins: Style.spacing.sm
              model: root.completions
              currentIndex: root.completionIndex
              interactive: false
              clip: true
              onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)

              delegate: Rectangle {
                required property var modelData
                required property int index
                readonly property bool current: index === root.completionIndex
                width: list.width
                height: Style.spacing.popupRowHeight
                color: current ? Style.selectedAccentFill : "transparent"
                radius: Style.cornerRadius

                MouseArea {
                  anchors.fill: parent
                  onClicked: root.completionPicked(index)
                }

                RowLayout {
                  anchors.fill: parent
                  anchors.leftMargin: Style.spacing.md
                  anchors.rightMargin: Style.spacing.md
                  spacing: Style.spacing.md

                  Text {
                    Layout.preferredWidth: Math.round(parent.width * 0.42)
                    Layout.maximumWidth: Math.round(parent.width * 0.42)
                    text: modelData.label
                    color: current ? Commons.Color.menu.selectedText : Commons.Color.menu.text
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                    elide: Text.ElideRight
                  }
                  Text {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    text: modelData.detail || ""
                    color: Commons.Color.muted
                    font.family: Style.font.family
                    font.pixelSize: Style.font.bodySmall
                    elide: Text.ElideRight
                  }
                }
              }
            }
          }

        // Underline under the offending span, drawn in the field's own coordinates.
        Rectangle {
          visible: root.open && root.errorStart >= 0
          readonly property real x0: input.positionToRectangle(Math.min(Math.max(0, root.errorStart), input.text.length)).x
          readonly property real x1: input.positionToRectangle(Math.min(Math.max(root.errorEnd, root.errorStart + 1), input.text.length)).x
          x: x0
          width: Math.max(Style.space(4), x1 - x0)
          y: input.height - Style.space(2)
          height: Style.space(2)
          color: Commons.Color.urgent
          z: 6
        }
      }
    }

    // Hint row: the live parse state (matched count and canonical form, or the parse error),
    // or the command help. Fixed height, one line, elided.
    Item {
      Layout.fillWidth: true
      Layout.preferredHeight: root.hintHeight

      Text {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: Style.spacing.sm
        text: root.hint
        color: root.hintError ? Commons.Color.urgent : Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        elide: Text.ElideRight
      }
    }
  }


}
