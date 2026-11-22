import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "../keys/Keymap.js" as Keymap
import "../state/Query.js" as Query

// Modal dialog for a transaction's note. Enter on a row opens it. It shows which transaction is
// being edited (payee on the first line, date / amount / account under it), a one-line field with
// a live character counter (the column is CHAR(255)), and Save / Cancel buttons.
//
// Same contract as InputPrompt: the owner calls show(initial), handles submitted(text), then
// close() on success or fail(msg) to keep the dialog open with the server's message. Validation is
// Query.parseNote, the same rules the backend applies, so a bad note never leaves the dialog.
Item {
  id: root

  property string title: "Note"
  property string payee: ""      // first context line (bold)
  property string detail: ""     // second context line (muted)
  property string footnote: ""   // extra explanation (several rows: what will be replaced)
  property bool open: false
  property bool busy: false
  property string error: ""

  signal submitted(string text)
  signal cancelled()

  readonly property int limit: Query.NOTE_MAX
  readonly property int used: Array.from(input.text).length
  readonly property bool over: used > limit

  visible: open
  z: 22

  function show(initial) {
    input.text = initial || ""
    error = ""
    busy = false
    open = true
    input.forceActiveFocus()
    input.selectAll()
  }

  function submit() {
    if (busy) return
    var r = Query.parseNote(input.text)
    if (r.error) { error = r.error; return }
    submitted(input.text)
  }

  function close() {
    if (!open) return
    open = false
    busy = false
  }

  function fail(msg) {
    error = msg
    busy = false
    input.forceActiveFocus()
  }

  function cancel() {
    if (!open || busy) return
    open = false
    cancelled()
  }

  Rectangle { anchors.fill: parent; color: Commons.Color.menu.scrim }
  MouseArea { anchors.fill: parent; onClicked: root.cancel() }

  Rectangle {
    id: card
    anchors.horizontalCenter: parent.horizontalCenter
    y: Math.round(parent.height * 0.18)
    width: Math.min(parent.width - Style.space(40), Style.space(620))
    height: col.implicitHeight + Style.spacing.lg * 2
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: root.over || root.error !== "" ? Commons.Color.urgent : Commons.Color.menu.border
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

      // Which transaction (hidden for a multi-row edit, which uses the footnote instead).
      Rectangle {
        visible: root.payee !== ""
        Layout.fillWidth: true
        implicitHeight: ctx.implicitHeight + Style.spacing.md * 2
        color: "transparent"
        border.width: Style.space(1)
        border.color: Commons.Color.menu.border
        radius: Style.cornerRadius

        ColumnLayout {
          id: ctx
          anchors.fill: parent
          anchors.margins: Style.spacing.md
          spacing: Style.space(2)

          Text {
            Layout.fillWidth: true
            text: root.payee
            elide: Text.ElideRight
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.body
            font.bold: true
          }
          Text {
            Layout.fillWidth: true
            text: root.detail
            elide: Text.ElideRight
            color: Commons.Color.muted
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
        }
      }

      Text {
        visible: root.footnote !== ""
        Layout.fillWidth: true
        text: root.footnote
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.WordWrap
      }

      TextField {
        id: input
        Layout.fillWidth: true
        enabled: !root.busy
        placeholderText: "Add a note\u2026"
        Keys.priority: Keys.BeforeItem
        onTextEdited: root.error = ""
        Keys.onPressed: function(e) {
          var id = Keymap.resolve(["notes"], Keymap.token(e))
          if (id === "notes.submit") { root.submit(); e.accepted = true }
          else if (id === "notes.cancel") { root.cancel(); e.accepted = true }
          else if (id === "notes.clear") { input.text = ""; root.error = ""; e.accepted = true }
        }
      }

      RowLayout {
        Layout.fillWidth: true
        spacing: Style.spacing.md

        Text {
          Layout.fillWidth: true
          visible: root.error !== ""
          text: root.error
          color: Commons.Color.urgent
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }
        Item { Layout.fillWidth: true; visible: root.error === "" }

        Text {
          text: root.used + " / " + root.limit
          color: root.over ? Commons.Color.urgent
               : root.used > root.limit - 25 ? Commons.Color.accent : Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }

      RowLayout {
        Layout.fillWidth: true
        spacing: Style.spacing.md

        Text {
          Layout.fillWidth: true
          text: root.busy ? "Saving\u2026"
                          : "Enter save   Esc cancel   Ctrl-U clear   empty removes the note   u undoes"
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
        }

        Rectangle {
          implicitWidth: cancelLabel.implicitWidth + Style.spacing.lg * 2
          implicitHeight: cancelLabel.implicitHeight + Style.spacing.sm * 2
          radius: Style.cornerRadius
          color: "transparent"
          border.width: Style.space(1)
          border.color: Commons.Color.menu.border
          Text {
            id: cancelLabel
            anchors.centerIn: parent
            text: "Cancel"
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
          MouseArea { anchors.fill: parent; onClicked: root.cancel() }
        }

        Rectangle {
          implicitWidth: saveLabel.implicitWidth + Style.spacing.lg * 2
          implicitHeight: saveLabel.implicitHeight + Style.spacing.sm * 2
          radius: Style.cornerRadius
          color: root.over || root.busy ? "transparent" : Commons.Color.accent
          border.width: Style.space(1)
          border.color: root.over || root.busy ? Commons.Color.menu.border : Commons.Color.accent
          opacity: root.over || root.busy ? 0.5 : 1
          Text {
            id: saveLabel
            anchors.centerIn: parent
            text: "Save"
            color: root.over || root.busy ? Commons.Color.menu.text : Commons.Color.menu.background
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
            font.bold: true
          }
          MouseArea { anchors.fill: parent; onClicked: root.submit() }
        }
      }
    }
  }
}
