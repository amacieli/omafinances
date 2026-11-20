import QtQuick
import qs.Commons
import qs.Commons as Commons

// One muted line at the bottom of the window for feedback on keys that cannot act yet, or
// confirmations. Fades out by itself. Never takes keys or mouse.
Rectangle {
  id: root

  property alias text: label.text
  property bool shown: false

  function show(message) {
    label.text = message
    shown = true
    hideTimer.restart()
  }

  visible: opacity > 0
  opacity: shown ? 1 : 0
  Behavior on opacity { NumberAnimation { duration: Style.duration(160) } }
  enabled: false
  z: 30
  height: label.implicitHeight + Style.spacing.md * 2
  width: Math.min(parent ? parent.width - Style.space(40) : 0, label.implicitWidth + Style.spacing.xl * 2)
  color: Commons.Color.menu.background
  border.width: Style.space(1)
  border.color: Commons.Color.menu.border
  radius: Style.cornerRadius

  Timer {
    id: hideTimer
    interval: 3000
    onTriggered: root.shown = false
  }

  Text {
    id: label
    anchors.centerIn: parent
    width: root.width - Style.spacing.xl * 2
    horizontalAlignment: Text.AlignHCenter
    wrapMode: Text.WordWrap
    color: Commons.Color.muted
    font.family: Style.font.family
    font.pixelSize: Style.font.bodySmall
  }
}
