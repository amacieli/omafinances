import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "../keys/Keymap.js" as Keymap

// One-line typed input overlay (amount, date range, text). The owner passes `validator`: a
// function(text) that returns "" when the text is acceptable, or a short message to show inline.
// Enter submits only an acceptable text; a bad one stays open with the message. Esc cancels.
Item {
  id: root

  property string title: ""
  property string hint: ""
  property string placeholder: ""
  property var validator: function(t) { return "" }
  property bool open: false

  signal submitted(string text)
  signal cancelled()

  property string error: ""

  visible: open
  z: 21

  function show(initial) {
    input.text = initial || ""
    error = ""
    open = true
    input.forceActiveFocus()
    input.cursorPosition = input.text.length
  }

  function submit() {
    var msg = validator(input.text)
    if (msg !== "") { error = msg; return }
    open = false
    submitted(input.text)
  }

  function cancel() {
    if (!open) return
    open = false
    cancelled()
  }

  Rectangle { anchors.fill: parent; color: Commons.Color.menu.scrim }
  MouseArea { anchors.fill: parent; onClicked: root.cancel() }

  Rectangle {
    anchors.horizontalCenter: parent.horizontalCenter
    y: Math.round(parent.height * 0.12)
    width: Math.min(parent.width - Style.space(40), Style.space(520))
    height: col.implicitHeight + Style.spacing.lg * 2
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: Commons.Color.menu.border
    radius: Style.cornerRadius

    MouseArea { anchors.fill: parent }

    ColumnLayout {
      id: col
      anchors.fill: parent
      anchors.margins: Style.spacing.lg
      spacing: Style.spacing.md

      Text {
        Layout.fillWidth: true
        text: root.title
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }

      Text {
        visible: root.hint !== ""
        Layout.fillWidth: true
        text: root.hint
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.WordWrap
      }

      TextField {
        id: input
        Layout.fillWidth: true
        placeholderText: root.placeholder
        Keys.priority: Keys.BeforeItem
        onTextEdited: root.error = ""
        Keys.onPressed: function(e) {
          var id = Keymap.resolve(["prompt"], Keymap.token(e))
          if (id === "prompt.submit") { root.submit(); e.accepted = true }
          else if (id === "prompt.cancel") { root.cancel(); e.accepted = true }
        }
      }

      Text {
        visible: root.error !== ""
        Layout.fillWidth: true
        text: root.error
        color: Commons.Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.WordWrap
      }

      Text {
        Layout.fillWidth: true
        text: "Enter apply   Esc cancel   empty clears the filter"
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }
    }
  }
}
