import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../keys/Keymap.js" as Keymap
import "../state/Insight.js" as Insight

// Review queue (R): one entry per payee group of uncategorized rows. A group shows its payee,
// count, total and date span, the pending category (y applies it to the ticked rows), its numbered
// suggestion chips (1/2/3) and how many rows will be categorized. Enter shows the group's rows so
// single rows can be ticked (Space). Movement and the cursor are this view's; every other key is
// forwarded as act(id). AppShell owns the requests and every piece of state.
FocusScope {
  id: root

  property bool open: false
  property var groups: []             // loaded GET /review rows, one per payee group, in queue order
  property string mode: ""            // "suggest" | "frequency" | "" (not loaded yet)
  property int remaining: 0           // groups the queue still holds
  property int startRemaining: 0      // remaining when the queue was opened (progress)
  property bool loading: false
  property string error: ""           // the backend's error envelope, shown verbatim
  property var suggState: null        // {enabled, reason, accuracy} from /suggest/accuracy, or null (not loaded)
  property string suggError: ""       // the backend's error envelope for the accuracy request, or ""
  property var pending: ({})          // group key -> {category_id, path}
  property var unticked: ({})         // group key -> {id: true}
  property var expanded: ({})         // group key -> true
  property var rowsByKey: ({})        // group key -> expanded rows loaded so far
  property var loadingByKey: ({})     // group key -> true while its rows load

  readonly property bool suggOn: suggState !== null && suggState.enabled === true
  // Flat list of headers, rows and "more" entries (Insight.groupItems).
  readonly property var items: Insight.groupItems(groups, expanded, rowsByKey, loadingByKey)
  property int cursorIdx: 0
  property string cursorKey: ""
  readonly property var cur: cursorIdx >= 0 && cursorIdx < items.length ? items[cursorIdx] : null
  readonly property var curGroup: cur ? cur.g : null
  readonly property int groupIdx: curGroup ? Insight.groupIndexOf(groups, cur.gkey) : 0
  readonly property var head: Insight.reviewHeader({ mode: mode, acc: suggState, remaining: remaining,
                                                     index: groupIdx, loaded: groups.length })
  readonly property real progress: Insight.reviewProgress(startRemaining, remaining)
  readonly property color warnColor: Commons.Color.urgent

  signal act(string id)

  visible: open
  z: 18

  // Takes the keyboard (AppShell calls this when the queue opens or an overlay above it closes).
  function focusKeys() { keyCatcher.forceActiveFocus() }

  // The cursor keeps its key while the items change; a group that left lands on the next one.
  onItemsChanged: {
    cursorIdx = Insight.cursorIndex(items, cursorKey, cursorIdx)
    cursorKey = items.length > 0 ? items[cursorIdx].key : ""
  }
  onOpenChanged: if (open) { cursorKey = ""; cursorIdx = 0 }

  function moveTo(i) {
    if (items.length === 0) return
    cursorIdx = Insight.clamp(i, items.length)
    cursorKey = items[cursorIdx].key
    list.positionViewAtIndex(cursorIdx, ListView.Contain)
    // near the end of the loaded groups: AppShell fetches the next page (it checks there is one)
    if (cur && cur.kind === "group" && groupIdx >= groups.length - 2) act("review.more")
  }

  // Movement is handled here; returns false for the other ids.
  function move(id) {
    if (id === "review.down") moveTo(cursorIdx + 1)
    else if (id === "review.up") moveTo(cursorIdx - 1)
    else if (id === "review.skip") moveTo(Insight.groupStep(items, cursorIdx, 1))
    else if (id === "review.prev") moveTo(Insight.groupStep(items, cursorIdx, -1))
    else return false
    return true
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

  Item {
    id: keyCatcher
    objectName: "reviewKeys"
    anchors.fill: parent
    focus: true
    Keys.onPressed: function(e) {
      var id = Keymap.resolve(["review"], Keymap.token(e))
      if (id === "") return
      e.accepted = true
      if (!root.move(id)) root.act(id)
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
        text: root.remaining > 0 ? Insight.fmtN(root.remaining) + (root.remaining === 1 ? " group" : " groups") + " to review" : ""
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

    // The groups, their expanded rows and the "more" lines.
    ListView {
      id: list
      Layout.fillWidth: true
      Layout.fillHeight: true
      visible: root.items.length > 0
      clip: true
      spacing: Style.spacing.xs
      model: root.items
      boundsBehavior: Flickable.StopAtBounds
      currentIndex: root.cursorIdx

      delegate: Rectangle {
        id: entry
        required property var modelData
        required property int index
        readonly property bool isCursor: index === root.cursorIdx
        width: list.width
        height: entry.modelData.kind === "group" ? body.implicitHeight + Style.spacing.md * 2
              : Style.space(20)
        color: isCursor ? Qt.alpha(Commons.Color.accent, 0.10) : "transparent"
        radius: Style.cornerRadius

        // A group header: the card with payee, pending category, chips and the tick count.
        ColumnLayout {
          id: body
          visible: entry.modelData.kind === "group"
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.top: parent.top
          anchors.margins: Style.spacing.md
          spacing: Style.spacing.xs

          readonly property var pend: entry.modelData.kind === "group"
              ? Insight.pendingFor(root.pending, entry.modelData.g, root.suggOn) : null
          readonly property var chipList: entry.modelData.kind === "group"
              ? Insight.optionChips(entry.modelData.g.options) : []

          Text {
            Layout.fillWidth: true
            text: entry.modelData.kind === "group" ? Insight.groupLine(entry.modelData.g) : ""
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.body
            font.bold: true
            elide: Text.ElideRight
          }
          Text {
            Layout.fillWidth: true
            text: Insight.pendingText(body.pend)
            color: body.pend ? Commons.Color.accent : root.warnColor
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
            elide: Text.ElideRight
          }
          // Numbered suggestion chips; the pending one is filled.
          Flow {
            Layout.fillWidth: true
            spacing: Style.spacing.md
            visible: body.chipList.length > 0
            Repeater {
              model: body.chipList
              delegate: Rectangle {
                required property var modelData
                readonly property bool isPending: body.pend !== null
                                                  && Number(body.pend.category_id) === Number(modelData.category_id)
                implicitWidth: chipLine.implicitWidth + Style.spacing.md * 2
                implicitHeight: chipLine.implicitHeight + Style.spacing.xs * 2
                radius: Style.cornerRadius
                color: isPending ? Commons.Color.accent : Qt.alpha(Commons.Color.accent, 0.12)
                border.width: Style.space(1)
                border.color: Commons.Color.accent
                Text {
                  id: chipLine
                  anchors.centerIn: parent
                  text: modelData.n + "  " + modelData.text
                  color: parent.isPending ? Commons.Color.menu.background : Commons.Color.menu.text
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                }
              }
            }
          }
          Text {
            Layout.fillWidth: true
            visible: entry.modelData.kind === "group" && body.chipList.length === 0
            text: "No suggestion for this payee. c picks a category."
            color: Commons.Color.muted
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
          Text {
            Layout.fillWidth: true
            text: entry.modelData.kind === "group"
                ? Insight.willText(entry.modelData.g, root.unticked) + "      "
                  + (root.expanded[entry.modelData.gkey] ? "Enter hides the rows" : "Enter shows the rows")
                : ""
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
        }

        // A transaction of an expanded group: [x] ticked, [ ] not.
        Text {
          visible: entry.modelData.kind === "row"
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          anchors.leftMargin: Style.spacing.xl
          anchors.rightMargin: Style.spacing.md
          text: entry.modelData.kind === "row"
              ? (Insight.isTicked(root.unticked, entry.modelData.gkey, Number(entry.modelData.row.id)) ? "[x]  " : "[ ]  ")
                + Insight.rowLine(entry.modelData.row)
              : ""
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          elide: Text.ElideRight
        }

        // The rest of an expanded group: Enter loads the next page.
        Text {
          visible: entry.modelData.kind === "more"
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          anchors.leftMargin: Style.spacing.xl
          text: entry.modelData.kind === "more"
              ? (entry.modelData.loading ? "loading rows..."
                 : "Enter loads " + Insight.fmtN(Math.min(entry.modelData.left, Insight.REVIEW_PAGE))
                   + " more (" + Insight.fmtN(entry.modelData.left) + " not shown yet)")
              : ""
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }
    }

    Text {
      Layout.fillWidth: true
      Layout.fillHeight: true
      visible: root.items.length === 0
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
      visible: root.items.length > 0 && root.error !== ""
      text: root.error
      color: root.warnColor
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      Layout.fillWidth: true
      text: "j/k move   1 2 3 suggestion   c category   C preview   y apply   Space tick   a all   n none   Enter rows   s skip   Backspace back   u undo   Esc collapse, then leave"
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      wrapMode: Text.WordWrap
    }
  }
}
