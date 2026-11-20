import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "../keys/Keymap.js" as Keymap
import "Fuzzy.js" as Fuzzy

// Keyboard picker overlay, generic: a list of {key, label, detail}. Type to filter (fuzzy).
// j/k move while the filter is empty (typing a j or k then filters); Down/Up and Ctrl-n/Ctrl-p
// always move; Enter chooses; Esc cancels. Keys are read from the "picker" scope of the keymap.
// Owns the keyboard while open; the parent must not handle keys while `open`.
Item {
  id: root

  property string title: ""
  property var entries: []          // [{key, label, detail}]
  property string hint: ""          // optional, several lines allowed
  property string currentKey: ""    // marked with a check when present
  property string emptyText: "no matching choice"
  property bool open: false

  readonly property var shown: Fuzzy.filterEntries(entries, input.text)

  signal chosen(string key)
  signal cancelled()

  visible: open
  z: 20

  function indexOfKey(k) {
    for (var i = 0; i < shown.length; i++) if (shown[i].key === k) return i
    return -1
  }

  function show() {
    input.text = ""
    open = true
    var i = indexOfKey(currentKey)
    list.currentIndex = i >= 0 ? i : 0
    input.forceActiveFocus()
  }

  function choose() {
    if (list.currentIndex < 0 || list.currentIndex >= shown.length) return
    var k = shown[list.currentIndex].key
    open = false
    chosen(k)
  }

  function cancel() {
    if (!open) return
    open = false
    cancelled()
  }

  function move(n) {
    if (shown.length === 0) return
    list.currentIndex = Math.max(0, Math.min(shown.length - 1, list.currentIndex + n))
  }

  // Dim the window behind the card; a click outside cancels.
  Rectangle { anchors.fill: parent; color: Commons.Color.menu.scrim }
  MouseArea { anchors.fill: parent; onClicked: root.cancel() }

  Rectangle {
    id: card
    anchors.horizontalCenter: parent.horizontalCenter
    y: Math.round(parent.height * 0.12)
    width: Math.min(parent.width - Style.space(40), Style.space(520))
    height: Math.min(parent.height - y - Style.space(40), contentCol.implicitHeight + Style.spacing.lg * 2)
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: Commons.Color.menu.border
    radius: Style.cornerRadius

    MouseArea { anchors.fill: parent }   // swallow clicks on the card

    ColumnLayout {
      id: contentCol
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
        elide: Text.ElideRight
      }

      TextField {
        id: input
        Layout.fillWidth: true
        placeholderText: "type to filter"
        // Owns its keys while focused: this runs before the field's own key handling.
        Keys.priority: Keys.BeforeItem
        onTextChanged: list.currentIndex = 0
        Keys.onPressed: function(e) {
          var t = Keymap.token(e)
          var id = Keymap.resolve(["picker"], t)
          // j and k are letters once there is something typed: they filter instead of moving.
          if ((id === "pick.down" || id === "pick.up") && (t === "j" || t === "k") && input.text !== "") return
          if (id === "pick.down") { root.move(1); e.accepted = true }
          else if (id === "pick.up") { root.move(-1); e.accepted = true }
          else if (id === "pick.choose") { root.choose(); e.accepted = true }
          else if (id === "pick.cancel") { root.cancel(); e.accepted = true }
        }
      }

      ListView {
        id: list
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: 0
        implicitHeight: Math.min(contentHeight, Style.space(420))
        clip: true
        interactive: contentHeight > height
        boundsBehavior: Flickable.StopAtBounds
        model: root.shown
        currentIndex: 0
        onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)

        delegate: Rectangle {
          required property var modelData
          required property int index
          readonly property bool current: index === list.currentIndex
          width: list.width
          height: Style.spacing.popupRowHeight
          color: current ? Style.selectedAccentFill : "transparent"

          MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            onEntered: list.currentIndex = index
            onClicked: { list.currentIndex = index; root.choose() }
          }

          RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Style.spacing.md
            anchors.rightMargin: Style.spacing.md
            spacing: Style.spacing.md

            Text {
              Layout.preferredWidth: Style.space(14)
              text: modelData.key === root.currentKey ? "\u2713" : ""
              color: Commons.Color.accent
              font.family: Style.font.family
              font.pixelSize: Style.font.body
            }
            Text {
              Layout.fillWidth: true
              Layout.minimumWidth: 0
              text: modelData.label
              color: current ? Commons.Color.menu.selectedText : Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              elide: Text.ElideRight
            }
            Text {
              Layout.maximumWidth: root.width * 0.5
              text: modelData.detail || ""
              color: Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
          }
        }
      }

      Text {
        visible: root.shown.length === 0
        Layout.fillWidth: true
        text: root.emptyText
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }

      Text {
        visible: root.hint !== ""
        Layout.fillWidth: true
        text: root.hint
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        Layout.fillWidth: true
        text: "type to filter   j/k or \u2191\u2193 move   Enter choose   Esc cancel"
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        elide: Text.ElideRight
      }
    }
  }
}
