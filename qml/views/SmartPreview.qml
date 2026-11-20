import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../keys/Keymap.js" as Keymap
import "../state/Smart.js" as Smart

// Categorize with preview (C): a full-pane overlay over the transaction window. It shows the
// clusters from POST /smart/preview: header with the counts and money that will be written,
// then each cluster (checkbox, tier, confidence, reason, status) with its exemplars, or with its
// rows when expanded (lazily, 50 at a time). Nothing is written from here: AppShell owns every
// request and every action. The overlay only moves the cursor and forwards the other keys as
// act(id). Owns the keyboard while open.
FocusScope {
  id: root

  property bool open: false
  property var clusters: []          // preview clusters, in response order
  property var summary: null         // preview summary
  property var selected: ({})        // cluster key -> bool (Smart.clusterKeys)
  property var expanded: ({})        // cluster key -> true
  property var rowsByKey: ({})       // cluster key -> rows loaded so far, request order
  property var loadingByKey: ({})    // cluster key -> true while /smart/rows is in flight
  property real threshold: 0.75
  property bool overwrite: false
  property bool makeRule: false
  property bool busy: false          // a preview or apply request is in flight
  property string error: ""          // the backend's message, or ""
  property string categoryPath: ""   // "Home › Maintenance"
  property string cursorKey: ""      // the cursor's item key; kept when the list changes

  // Conflicts and failures use the kit's alert color.
  readonly property color warnColor: Commons.Color.urgent
  readonly property real dimmed: 0.45
  readonly property int indentStep: Style.space(18)

  readonly property var clusterKeys: Smart.clusterKeys(clusters)
  readonly property var items: Smart.flatten(clusters, clusterKeys, expanded, rowsByKey, loadingByKey)
  readonly property int cursor: Smart.indexOfKey(items, cursorKey)
  readonly property var tallyNow: Smart.tally(clusters, selected, overwrite)
  readonly property string headerLine: clusters.length > 0 ? Smart.headerText(summary, tallyNow, overwrite) : ""
  readonly property bool beyondEmpty: clusters.length > 0 && Smart.nothingBeyondSeed(clusters)

  signal act(string id)

  visible: open
  z: 22

  // Takes the keyboard (AppShell calls this when the preview opens or an overlay above it closes).
  function focusKeys() { keyCatcher.forceActiveFocus() }

  // The cluster with this key, or null.
  function clusterFor(key) {
    var i = clusterKeys.indexOf(key)
    return i >= 0 ? clusters[i] : null
  }

  // The item under the cursor, or null.
  function currentItem() {
    return cursor >= 0 && cursor < items.length ? items[cursor] : null
  }

  function moveBy(dir, steps) {
    var i = Smart.moveStop(items, cursor, dir, steps)
    if (i >= 0 && i < items.length) cursorKey = items[i].key
  }

  function handle(id) {
    if (id === "smart.down") moveBy(1, 1)
    else if (id === "smart.up") moveBy(-1, 1)
    else if (id === "smart.pagedown") moveBy(1, 10)
    else if (id === "smart.pageup") moveBy(-1, 10)
    else act(id)
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

  // Keys live on this item; the children do not take focus.
  Item {
    id: keyCatcher
    objectName: "smartPreviewKeys"
    anchors.fill: parent
    focus: true
    Keys.onPressed: function(e) {
      var id = Keymap.resolve(["smart"], Keymap.token(e))
      if (id === "") return
      e.accepted = true
      root.handle(id)
    }
  }

  ColumnLayout {
    anchors.fill: parent
    anchors.margins: Style.spacing.xxl
    spacing: Style.spacing.md

    // Title: the action and the target category.
    RowLayout {
      Layout.fillWidth: true
      spacing: Style.spacing.lg * 2

      Text {
        text: "Categorize with preview"
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }
      Text {
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        text: root.categoryPath !== "" ? "\u2192 " + root.categoryPath : ""
        color: Commons.Color.accent
        font.family: Style.font.family
        font.pixelSize: Style.font.body
        elide: Text.ElideRight
      }
      Text {
        text: "threshold " + Smart.thresholdPercent(root.threshold)
              + "   overwrite " + (root.overwrite ? "on" : "off")
              + "   rule " + (root.makeRule ? "on" : "off")
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    // The header line: what applying the selection writes.
    Text {
      Layout.fillWidth: true
      visible: root.headerLine !== ""
      text: root.headerLine
      color: Commons.Color.menu.text
      font.family: Style.font.family
      font.pixelSize: Style.font.body
      font.bold: true
      elide: Text.ElideRight
    }

    Text {
      Layout.fillWidth: true
      visible: root.beyondEmpty
      text: "Nothing matched beyond the selected rows at " + Smart.thresholdPercent(root.threshold)
            + ". [ lowers the threshold to find more."
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    // The cluster list. Empty while the first preview is in flight, or after a failed one.
    Item {
      Layout.fillWidth: true
      Layout.fillHeight: true

      ListView {
        id: list
        anchors.fill: parent
        clip: true
        model: root.items
        currentIndex: root.cursor
        boundsBehavior: Flickable.StopAtBounds
        onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)

        delegate: Rectangle {
          id: line
          required property var modelData
          required property int index
          readonly property bool isCluster: modelData.kind === "cluster"
          readonly property bool isMore: modelData.kind === "more"
          readonly property bool current: index === root.cursor && (isCluster || isMore)
          readonly property var cl: root.clusterFor(modelData.ckey)
          readonly property bool isSelected: !!root.selected[modelData.ckey]
          readonly property string st: cl ? String(cl.status) : ""
          readonly property color tint: st === "conflict" ? root.warnColor : Commons.Color.menu.text

          width: list.width
          height: isCluster ? Style.spacing.popupRowHeight + Style.spacing.xs
                            : Style.space(22)
          color: current ? Style.selectedAccentFill : "transparent"

          // Cluster header: checkbox, label, tier, confidence, reason, the counts, the status.
          RowLayout {
            visible: line.isCluster
            anchors.fill: parent
            anchors.leftMargin: Style.spacing.md
            anchors.rightMargin: Style.spacing.md
            spacing: Style.spacing.md
            opacity: line.st === "same" ? root.dimmed : 1

            Text {
              text: line.isSelected ? "[x]" : "[ ]"
              color: line.tint
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              font.bold: line.isSelected
            }
            Text {
              Layout.maximumWidth: Style.space(220)
              text: line.cl ? String(line.cl.label) : ""
              color: line.tint
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              font.bold: true
              elide: Text.ElideRight
            }
            Text {
              text: line.cl ? (line.cl.tier + "  " + Math.round((Number(line.cl.confidence) || 0) * 100) + "%") : ""
              color: Commons.Color.muted
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
            }
            Text {
              Layout.fillWidth: true
              Layout.minimumWidth: 0
              text: line.cl ? Smart.clusterSummary(line.cl) : ""
              color: Commons.Color.menu.text
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
            Text {
              Layout.maximumWidth: Style.space(300)
              text: line.cl ? Smart.statusText(line.cl) : ""
              color: line.tint
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
          }

          // Under a cluster: the reason (muted), the exemplar rows, the loaded rows, "more".
          RowLayout {
            visible: !line.isCluster
            anchors.fill: parent
            anchors.leftMargin: Style.spacing.md + Style.space(24)
            anchors.rightMargin: Style.spacing.md
            spacing: Style.spacing.md
            opacity: line.st === "same" ? root.dimmed : 1

            Text {
              Layout.fillWidth: true
              Layout.minimumWidth: 0
              text: {
                var m = line.modelData
                if (m.kind === "more") {
                  if (m.loading) return "loading more rows..."
                  return "\u2026 " + m.left + " more  (Enter loads the next 50)"
                }
                var r = m.row || {}
                if (m.kind === "row") return Smart.rowLine(r)
                // exemplar: the row, its category now (if any) and why it was picked
                return Smart.rowLine(r) + (r.category_path ? "   now: " + r.category_path : "")
                       + "   (" + String(r.why || "") + ")"
              }
              color: line.modelData.kind === "more" ? Commons.Color.accent
                     : (line.st === "conflict" && line.modelData.kind === "exemplar" ? root.warnColor : Commons.Color.muted)
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
              elide: Text.ElideRight
            }
          }
        }
      }

      Text {
        anchors.centerIn: parent
        width: parent.width
        visible: root.clusters.length === 0
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        text: root.busy ? "Previewing..." : (root.error !== "" ? "" : "No preview yet.")
        color: Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
      }
    }

    Text {
      Layout.fillWidth: true
      visible: root.error !== ""
      text: root.error
      color: root.warnColor
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      Layout.fillWidth: true
      text: "j/k move   Space select   a all   n none   Enter expand   [ ] threshold   o overwrite   r rule   y apply   Esc cancel"
      color: Commons.Color.muted
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      elide: Text.ElideRight
    }
  }
}
