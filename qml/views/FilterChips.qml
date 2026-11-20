import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons

// Active filters as chips under the header. Click a chip to clear it; `X` clears the newest.
// `x` clears all. With no filters the row shows only the hint, so it never looks empty by accident.
RowLayout {
  id: root

  property var chips: []
  signal chipClicked(string id)

  spacing: Style.spacing.md

  Flow {
    id: flow
    Layout.fillWidth: true
    Layout.minimumWidth: 0
    spacing: Style.spacing.sm

    Repeater {
      model: root.chips
      delegate: Rectangle {
        required property var modelData
        width: chipText.implicitWidth + Style.spacing.md * 2 + closeText.implicitWidth + Style.spacing.sm
        height: chipText.implicitHeight + Style.spacing.xs * 2
        radius: Style.cornerRadius
        color: Qt.alpha(Commons.Color.accent, 0.14)
        border.width: Style.space(1)
        border.color: Qt.alpha(Commons.Color.accent, 0.5)

        Text {
          id: chipText
          anchors.left: parent.left
          anchors.leftMargin: Style.spacing.md
          anchors.verticalCenter: parent.verticalCenter
          text: modelData.label
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
        Text {
          id: closeText
          anchors.right: parent.right
          anchors.rightMargin: Style.spacing.md
          anchors.verticalCenter: parent.verticalCenter
          text: "\u00d7"
          color: Commons.Color.accent
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
        MouseArea {
          anchors.fill: parent
          onClicked: root.chipClicked(modelData.id)
        }
      }
    }
  }

  Text {
    Layout.alignment: Qt.AlignVCenter
    text: root.chips.length > 0 ? "x clears all   X clears newest" : "f filters"
    color: Commons.Color.muted
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
  }
}
