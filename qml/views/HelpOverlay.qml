import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../keys/Keymap.js" as Keymap

// `?`: every key, generated from the keymap registry and grouped by where it works. The groups
// for the pane that has focus are marked "here" and listed first. j/k scroll; Esc, q or ? close.
FocusScope {
  id: root

  property bool open: false
  property string focusedZone: "table"   // "table" | "sidebar" | "detail"

  signal closed()

  readonly property var allGroups: Keymap.groups()

  // The focused pane's groups first, then the rest in registry order.
  readonly property var orderedGroups: {
    var here = [], rest = []
    for (var i = 0; i < allGroups.length; i++) {
      var g = allGroups[i]
      var mine = g.scope === focusedZone || (focusedZone === "table" && g.scope === "table")
      var entry = { scope: g.scope, title: g.title + (mine ? "   \u2190 here" : ""), rows: g.rows }
      if (mine) here.push(entry)
      else rest.push(entry)
    }
    return here.concat(rest)
  }

  visible: open
  z: 25

  function show() {
    open = true
    flick.contentY = 0
    forceActiveFocus()
  }

  function hide() {
    if (!open) return
    open = false
    closed()
  }

  function scroll(dy) {
    flick.contentY = Math.max(0, Math.min(Math.max(0, flick.contentHeight - flick.height), flick.contentY + dy))
  }

  Rectangle { anchors.fill: parent; color: Commons.Color.menu.scrim }
  MouseArea { anchors.fill: parent; onClicked: root.hide() }

  Keys.priority: Keys.BeforeItem
  Keys.onPressed: function(e) {
    var id = Keymap.resolve(["help"], Keymap.token(e))
    if (id === "help.close") hide()
    else if (id === "help.down") scroll(40)
    else if (id === "help.up") scroll(-40)
    else if (e.key === Qt.Key_PageDown || e.key === Qt.Key_Space) scroll(flick.height * 0.9)
    else if (e.key === Qt.Key_PageUp) scroll(-flick.height * 0.9)
    else if (e.text === "g") flick.contentY = 0
    else if (e.text === "G") flick.contentY = Math.max(0, flick.contentHeight - flick.height)
    else return
    e.accepted = true
  }

  Rectangle {
    anchors.centerIn: parent
    width: Math.min(parent.width - Style.space(60), Style.space(860))
    height: parent.height - Style.space(80)
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: Commons.Color.menu.border
    radius: Style.cornerRadius

    MouseArea { anchors.fill: parent }

    Text {
      id: heading
      x: Style.spacing.xl
      y: Style.spacing.lg
      text: "Keyboard help"
      color: Commons.Color.accent
      font.family: Style.font.family
      font.pixelSize: Style.font.subtitle
      font.bold: true
    }
    Text {
      anchors.right: parent.right
      anchors.rightMargin: Style.spacing.xl
      y: Style.spacing.lg + Style.space(2)
      text: "j/k scroll   Esc close"
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
    }

    Flickable {
      id: flick
      anchors.fill: parent
      anchors.topMargin: heading.y + heading.height + Style.spacing.lg
      anchors.leftMargin: Style.spacing.xl
      anchors.rightMargin: Style.spacing.xl
      anchors.bottomMargin: Style.spacing.xl
      contentWidth: width
      contentHeight: body.implicitHeight
      clip: true
      boundsBehavior: Flickable.StopAtBounds

      Column {
        id: body
        width: flick.width
        spacing: Style.spacing.lg

        Repeater {
          model: root.orderedGroups
          delegate: Column {
            required property var modelData
            width: body.width
            spacing: Style.spacing.xs

            Text {
              text: modelData.title
              color: modelData.title.indexOf("\u2190 here") >= 0 ? Commons.Color.accent : Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              font.bold: true
            }
            Rectangle { width: parent.width; height: Style.space(1); color: Commons.Color.menu.border }
            Repeater {
              model: modelData.rows
              delegate: RowLayout {
                required property var modelData
                width: body.width
                spacing: Style.spacing.xl
                Text {
                  Layout.preferredWidth: Style.space(230)
                  Layout.maximumWidth: Style.space(230)
                  text: modelData.keys
                  color: Commons.Color.accent
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                  elide: Text.ElideRight
                }
                Text {
                  Layout.fillWidth: true
                  text: modelData.label
                  color: Commons.Color.menu.text
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                  wrapMode: Text.WordWrap
                }
              }
            }
          }
        }
      }
    }
  }
}
