import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../state/Query.js" as Query

// Right detail pane for the selected transaction (GET /transactions/<id>). Shows every field the
// backend returns, the Lunch Money category next to the local one, tags and notes.
// Read-only. Scrolling is driven by AppShell (scrollBy).
Rectangle {
  id: root

  property var txn: null          // parsed detail body, or null
  property bool loading: false
  property string error: ""
  property bool focusedPane: false

  function scrollBy(dy) {
    var max = Math.max(0, flick.contentHeight - flick.height)
    flick.contentY = Math.max(0, Math.min(max, flick.contentY + dy))
  }

  function orDash(v) {
    if (v === undefined || v === null || v === "") return "\u2014"
    return String(v)
  }

  function yesNo(v) { return v ? "yes" : "no" }

  // [{k, v}] in display order.
  readonly property var fieldRows: {
    if (!txn) return []
    var t = txn
    var amt = Query.fmtAmountRow(t.amount, t.currency)
    var direction = Number(t.amount) < 0 ? "money in" : "money out"
    var out = [
      { k: "Date", v: orDash(t.date) },
      { k: "Payee", v: orDash(t.payee) },
      { k: "Original name", v: orDash(t.original_name) },
      { k: "Amount", v: amt + "  (" + direction + ")" },
      { k: "Account", v: orDash(t.account) },
      { k: "Category (local)", v: orDash(t.category) },
      { k: "Category (Lunch Money)", v: orDash(t.lm_category) },
      { k: "Status", v: orDash(t.status) },
      { k: "Pending", v: yesNo(t.pending) },
      { k: "Recurring", v: t.recurring_id ? "yes (id " + t.recurring_id + ")" : yesNo(t.recurring) },
      { k: "Source", v: orDash(t.source) },
      { k: "External id", v: orDash(t.external_id) },
      { k: "Transaction id", v: orDash(t.id) },
      { k: "Created (LM)", v: orDash(t.created_at_lm) },
      { k: "Updated (LM)", v: orDash(t.updated_at_lm) }
    ]
    return out
  }

  color: "transparent"
  border.width: Style.space(1)
  border.color: focusedPane ? Commons.Color.menu.selectedBorder : Commons.Color.menu.border
  radius: Style.cornerRadius
  clip: true

  Text {
    id: title
    x: Style.spacing.xl
    y: Style.spacing.lg
    text: "DETAIL"
    color: Commons.Color.muted
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
    font.bold: true
  }

  Flickable {
    id: flick
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.top: title.bottom
    anchors.bottom: parent.bottom
    anchors.margins: Style.spacing.xl
    anchors.topMargin: Style.spacing.md
    contentWidth: width
    contentHeight: body.implicitHeight
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
      id: body
      width: flick.width
      spacing: Style.spacing.md

      Text {
        visible: root.txn === null
        width: parent.width
        wrapMode: Text.WordWrap
        text: root.error !== "" ? root.error
             : (root.loading ? "loading..." : "Select a transaction to see its details.")
        color: root.error !== "" ? Commons.Color.urgent : Commons.Color.muted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }

      Text {
        visible: root.txn !== null && root.txn.payee !== undefined
        width: parent.width
        wrapMode: Text.WordWrap
        text: root.txn ? root.orDash(root.txn.payee) : ""
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }

      Repeater {
        model: root.fieldRows
        delegate: Column {
          required property var modelData
          width: body.width
          spacing: Style.spacing.xxs
          Text {
            text: modelData.k.toUpperCase()
            color: Commons.Color.muted
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
          }
          Text {
            width: parent.width
            wrapMode: Text.WrapAnywhere
            text: modelData.v
            color: Commons.Color.menu.text
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
        }
      }

      Column {
        visible: root.txn !== null && Array.isArray(root.txn.tags) && root.txn.tags.length > 0
        width: body.width
        spacing: Style.spacing.xxs
        Text {
          text: "TAGS"
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
        }
        Flow {
          width: parent.width
          spacing: Style.spacing.sm
          Repeater {
            model: root.txn && Array.isArray(root.txn.tags) ? root.txn.tags : []
            delegate: Rectangle {
              required property var modelData
              width: tagText.implicitWidth + Style.spacing.md * 2
              height: tagText.implicitHeight + Style.spacing.xxs * 2
              radius: Style.cornerRadius
              color: Qt.alpha(Commons.Color.accent, 0.12)
              Text {
                id: tagText
                anchors.centerIn: parent
                text: String(modelData)
                color: Commons.Color.accent
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
              }
            }
          }
        }
      }

      Column {
        visible: root.txn !== null && !!root.txn.notes
        width: body.width
        spacing: Style.spacing.xxs
        Text {
          text: "NOTES"
          color: Commons.Color.muted
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
        }
        Text {
          width: parent.width
          wrapMode: Text.WordWrap
          text: root.txn ? String(root.txn.notes) : ""
          color: Commons.Color.menu.text
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }
    }
  }
}
