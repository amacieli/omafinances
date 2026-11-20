import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../keys/Keymap.js" as Keymap
import "../state/Insight.js" as Insight

// Review queue (R): one uncategorized row at a time, with its suggestions as numbered chips 1/2/3.
// Accepting (1-3), the category picker (c) and the preview (C, bootstrap mode only) are AppShell's;
// this view renders the state it is given and forwards its keys as act(id). Owns the keyboard while open.
FocusScope {
  id: root

  property bool open: false
  property var rows: []               // loaded GET /review rows, in queue order
  property string mode: ""            // "suggest" | "frequency" | "" (not loaded yet)
  property int remaining: 0           // uncategorized rows the queue still holds
  property int startRemaining: 0      // remaining when the queue was opened (progress)
  property int cursor: 0
  property bool loading: false
  property string error: ""           // the backend's error envelope, shown verbatim
  property var suggState: null        // {enabled, reason, accuracy} from /suggest/accuracy, or null (not loaded)
  property string suggError: ""       // the backend's error envelope for the accuracy request, or ""

  readonly property var row: cursor >= 0 && cursor < rows.length ? rows[cursor] : null
  readonly property var chips: row ? Insight.optionChips(row.options) : []
  readonly property var head: Insight.reviewHeader({ mode: mode, acc: suggState, remaining: remaining,
                                                     index: cursor, loaded: rows.length })
  readonly property real progress: Insight.reviewProgress(startRemaining, remaining)
  readonly property color warnColor: Commons.Color.urgent

  signal act(string id)

  visible: open
  z: 18

  // Takes the keyboard (AppShell calls this when the queue opens or an overlay above it closes).
  function focusKeys() { keyCatcher.forceActiveFocus() }

  // Swallows clicks so the table underneath does not react.
  MouseArea { anchors.fill: parent }

  Rectangle {
    anchors.fill: parent
    color: Commons.Color.menu.background
    border.width: Style.space(1)
    border.color: Commons.Color.menu.border
    radius: Style.cornerRadius
  }

  Item {
    id: keyCatcher
    objectName: "reviewKeys"
    anchors.fill: parent
    focus: true
    Keys.onPressed: function(e) {
      var id = Keymap.resolve(["review"], Keymap.token(e))
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
        text: "Review queue"
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }
      Item { Layout.fillWidth: true }
      Text {
        text: root.remaining > 0 ? Insight.fmtN(root.remaining) + " to review" : ""
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    // Mode line: which rule the queue is sorted by, and why.
    Text {
      Layout.fillWidth: true
      text: root.suggError !== "" ? root.suggError : root.head.mode
      color: root.suggError !== "" ? root.warnColor : Commons.Color.menu.text
      font.family: Style.font.family
      font.pixelSize: Style.font.body
      font.bold: true
      wrapMode: Text.WordWrap
    }
    Text {
      Layout.fillWidth: true
      visible: root.head.detail !== "" && root.suggError === ""
      text: root.head.detail
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    // Progress: how much of the queue as it was when opened is handled.
    Rectangle {
      Layout.fillWidth: true
      implicitHeight: Style.space(4)
      color: Commons.Color.menu.border
      Rectangle {
        height: parent.height
        width: parent.width * root.progress
        color: Commons.Color.accent
      }
    }
    Text {
      Layout.fillWidth: true
      text: root.head.progress
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
    }

    // The current row.
    Rectangle {
      Layout.fillWidth: true
      Layout.fillHeight: true
      visible: root.row !== null
      color: "transparent"
      border.width: Style.space(1)
      border.color: Commons.Color.menu.border
      radius: Style.cornerRadius

      ColumnLayout {
        anchors.fill: parent
        anchors.margins: Style.spacing.xl
        spacing: Style.spacing.md

        Text {
          Layout.fillWidth: true
          text: root.row ? String(root.row.payee) : ""
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.subtitle
          font.bold: true
          elide: Text.ElideRight
        }
        Text {
          Layout.fillWidth: true
          text: root.row ? String(root.row.date) + "   " + String(root.row.account) + "   "
                            + Insight.moneyCents(root.row.amount) : ""
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.body
        }
        Text {
          Layout.fillWidth: true
          text: root.row ? Insight.fmtN(root.row.payee_count) + " uncategorized rows from this payee" : ""
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }

        // Numbered chips: 1 accepts the first, 2 the second, 3 the third.
        Repeater {
          model: root.chips
          delegate: Rectangle {
            required property var modelData
            Layout.fillWidth: true
            implicitHeight: chipText.implicitHeight + Style.spacing.md * 2
            radius: Style.cornerRadius
            color: Qt.alpha(Commons.Color.accent, 0.12)
            border.width: Style.space(1)
            border.color: Commons.Color.accent
            RowLayout {
              anchors.fill: parent
              anchors.margins: Style.spacing.md
              spacing: Style.spacing.md
              Text {
                text: String(modelData.n)
                color: Commons.Color.accent
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
              }
              Text {
                id: chipText
                Layout.fillWidth: true
                text: modelData.path + "   " + Insight.pct(modelData.confidence) + "   " + modelData.source
                color: Commons.Color.menu.text
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                elide: Text.ElideRight
              }
            }
          }
        }
        Text {
          Layout.fillWidth: true
          visible: root.row !== null && root.chips.length === 0
          text: "No suggestion for this row. s skips it, c picks a category."
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }
        Text {
          Layout.fillWidth: true
          visible: root.row !== null && root.mode === "frequency"
          text: "C previews this whole payee group and clears it in one step."
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }
        Item { Layout.fillHeight: true }
      }
    }

    Text {
      Layout.fillWidth: true
      visible: root.row === null
      text: root.loading ? "loading..."
          : root.error !== "" ? root.error
          : "Queue is empty: nothing uncategorized."
      color: root.error !== "" ? root.warnColor : Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.subtitle
      wrapMode: Text.WordWrap
    }

    Text {
      Layout.fillWidth: true
      visible: root.row !== null && root.error !== ""
      text: root.error
      color: root.warnColor
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      Layout.fillWidth: true
      text: "1 2 3 accept   c categorize   C preview payee group (bootstrap)   s skip   Backspace back   u undo   Esc leave"
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      elide: Text.ElideRight
    }
  }
}
