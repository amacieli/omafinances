import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as Commons
import "../state"
import "../keys/Keymap.js" as Keymap
import "../state/Reports.js" as Rep

// Reports overlay (P): the saved reports and their email schedule (Phase 6). Modes: list, edit
// (new or existing), preview, runs, confirm (delete). The view owns its requests and state; text
// fields are typed through AppShell's input prompt (askText / promptResult). Error envelopes are
// shown verbatim; 409 not_authenticated adds the `omafinances gmail-auth` hint.
FocusScope {
  id: root

  property bool open: false
  property string currentQuery: ""     // the table's current query (default query of a new report)
  property string mode: "list"         // list | edit | preview | runs | confirm
  property var reports: []
  property string listError: ""
  property bool loading: false
  property var mailStatus: null
  property string mailError: ""
  property int cursorIdx: 0
  property string notice: ""           // last action result
  property bool noticeError: false
  property var form: null              // the report being edited
  property int fieldIdx: 0
  property string formError: ""
  property var nextRuns: []
  property string nextOncal: ""
  property string nextError: ""
  property var previewData: null
  property string previewError: ""
  property var runs: []
  property string runsError: ""
  property int gen: 0
  property int nextGen: 0
  property bool busy: false

  readonly property var cur: cursorIdx >= 0 && cursorIdx < reports.length ? reports[cursorIdx] : null
  readonly property var fields: form ? Rep.editFields(form) : []
  readonly property string scope: mode === "edit" ? "reportsEdit" : (mode === "preview" || mode === "runs") ? "reportsView"
                                  : mode === "confirm" ? "reportsConfirm" : "reports"

  signal askText(string ctx, string title, string hint, string placeholder, string initial)
  signal toast(string msg)
  signal closeRequested()

  visible: open
  z: 19

  function focusKeys() { keyCatcher.forceActiveFocus() }

  // ---- requests
  function envText(xhr) {
    var j = null
    try { j = JSON.parse(xhr.responseText) } catch (e) { j = null }
    if (xhr.status === 0) return "backend not responding (port " + Backend.port + ")"
    return Rep.envelopeText(xhr.status, j)
  }
  function body(xhr) { try { return JSON.parse(xhr.responseText) } catch (e) { return null } }

  function loadList() {
    var g = ++gen
    loading = true
    Backend.request("GET", "/reports", function(xhr) {
      if (g !== gen) return
      loading = false
      var j = xhr.status === 200 ? body(xhr) : null
      if (!j) { reports = []; listError = envText(xhr); return }
      listError = ""
      reports = j.reports || []
      cursorIdx = Rep.clamp(cursorIdx, reports.length)
    })
  }
  function loadStatus() {
    Backend.request("GET", "/mail/status", function(xhr) {
      var j = xhr.status === 200 ? body(xhr) : null
      if (!j) { mailStatus = null; mailError = envText(xhr); return }
      mailError = ""
      mailStatus = j
    })
  }

  // Called by AppShell when the overlay opens. newName !== "" starts a new report with that name.
  function show(newName) {
    mode = "list"
    notice = ""
    formError = ""
    loadList()
    loadStatus()
    if (newName !== undefined && newName !== null && newName !== "") startNew(newName)
  }
  function hide() { gen++; nextGen++ }

  function startNew(name) {
    form = Rep.newForm(currentQuery, mailStatus && mailStatus.sender ? mailStatus.sender : "", name || "")
    fieldIdx = 0
    formError = ""
    mode = "edit"
    loadNext()
  }
  function startEdit() {
    if (!cur) return
    form = Rep.formFromReport(cur)
    fieldIdx = 0
    formError = ""
    mode = "edit"
    loadNext()
  }

  function loadNext() {
    if (!form) return
    var g = ++nextGen
    var te = Rep.timeError(form.schedule.time)
    if (te) { nextRuns = []; nextError = te; return }
    Backend.request("GET", Rep.schedulePreviewPath(form.schedule), function(xhr) {
      if (g !== nextGen) return
      var j = xhr.status === 200 ? body(xhr) : null
      if (!j) { nextRuns = []; nextOncal = ""; nextError = envText(xhr); return }
      nextError = ""
      nextRuns = j.next_runs || []
      nextOncal = j.oncalendar || ""
    })
  }

  function saveForm() {
    var e = Rep.validateForm(form)
    if (e) { formError = e; return }
    if (busy) return
    busy = true
    formError = ""
    var f = form
    var isNew = f.id === null || f.id === undefined
    Backend.json(isNew ? "POST" : "PUT", isNew ? "/reports" : "/reports/" + f.id, Rep.formBody(f), function(xhr) {
      busy = false
      if (xhr.status !== 200) { formError = envText(xhr); return }
      mode = "list"
      form = null
      notice = isNew ? "created " + f.name : "saved " + f.name
      noticeError = false
      loadList()
    })
  }

  function postAct(path, payload, okText, after) {
    if (busy) return
    busy = true
    notice = ""
    Backend.json("POST", path, payload || {}, function(xhr) {
      busy = false
      if (xhr.status !== 200) { notice = envText(xhr); noticeError = true; return }
      var j = body(xhr)
      notice = typeof okText === "function" ? okText(j) : okText
      noticeError = false
      if (after) after(j)
      loadList()
    })
  }

  function sendTest() {
    if (!cur) return
    notice = "sending test..."; noticeError = false
    var r = cur
    postAct("/reports/" + r.id + "/send-test", {}, "test sent to " + (mailStatus && mailStatus.sender ? mailStatus.sender : "the sender"))
  }
  function toggleEnabled() {
    if (!cur) return
    var on = !cur.enabled
    postAct("/reports/" + cur.id + "/enable", { enabled: on }, on ? "enabled " + cur.name : "disabled " + cur.name)
  }
  function openPreview() {
    if (!cur) return
    previewData = null; previewError = ""
    mode = "preview"
    flick.contentY = 0
    busy = true
    Backend.json("POST", "/reports/" + cur.id + "/preview", {}, function(xhr) {
      busy = false
      var j = xhr.status === 200 ? body(xhr) : null
      if (!j) { previewError = envText(xhr); return }
      previewData = j
    })
  }
  function openRuns() {
    if (!cur) return
    runs = []; runsError = ""
    mode = "runs"
    flick.contentY = 0
    Backend.request("GET", "/reports/" + cur.id + "/runs?limit=20", function(xhr) {
      var j = xhr.status === 200 ? body(xhr) : null
      if (!j) { runsError = envText(xhr); return }
      runs = j.runs || []
    })
  }
  function doDelete() {
    if (!cur) return
    var r = cur
    mode = "list"
    Backend.request("DELETE", "/reports/" + r.id, function(xhr) {
      if (xhr.status !== 200 && xhr.status !== 204) { notice = envText(xhr); noticeError = true; return }
      notice = "deleted " + r.name; noticeError = false
      loadList()
    })
  }

  // ---- text fields (typed through AppShell's prompt)
  function editText(key) {
    var f = form
    if (key === "name") askText("name", "Report name", "Must be unique.", "name", f.name)
    else if (key === "query") askText("query", "Report query", "The slash-search grammar. Empty = all transactions. The period is applied on top.", "e.g. c:groceries", f.query)
    else if (key === "time") askText("time", "Run time", "24 hour, HH:MM, local time.", "07:30", f.schedule.time)
    else if (key === "recipients") askText("recipients", "Recipients", "Comma separated. Only the allowlist is accepted (default: the sender).", "me@example.com", (f.recipients || []).join(", "))
  }
  function promptResult(ctx, text) {
    if (!form) return ""
    var f = Object.assign({}, form, { schedule: Object.assign({}, form.schedule) })
    if (ctx === "name") f.name = text.trim()
    else if (ctx === "query") f.query = text.trim()
    else if (ctx === "time") {
      var te = Rep.timeError(text.trim())
      if (te) return te
      f.schedule.time = text.trim()
    } else if (ctx === "recipients") {
      var list = Rep.parseRecipients(text)
      var re = Rep.recipientsError(list)
      if (re) return re
      f.recipients = list
    }
    form = f
    formError = ""
    if (ctx === "time") loadNext()
    return ""
  }

  function adjust(dir) {
    var key = fields[fieldIdx]
    if (Rep.TEXT_FIELDS.indexOf(key) >= 0) return
    form = Rep.adjustField(form, key, dir)
    fieldIdx = Rep.clamp(fieldIdx, Rep.editFields(form).length)
    if (key === "kind" || key === "weekday" || key === "day") loadNext()
  }

  function scrollBy(d) { flick.contentY = Rep.scrollBy(flick.contentY, d, flick.contentHeight, flick.height) }

  function handle(id) {
    if (id === "reports.close") { hide(); closeRequested() }
    else if (id === "reports.down") cursorIdx = Rep.moveIndex(cursorIdx, 1, reports.length)
    else if (id === "reports.up") cursorIdx = Rep.moveIndex(cursorIdx, -1, reports.length)
    else if (id === "reports.new") startNew("")
    else if (id === "reports.edit") startEdit()
    else if (id === "reports.preview") openPreview()
    else if (id === "reports.test") sendTest()
    else if (id === "reports.toggle") toggleEnabled()
    else if (id === "reports.runs") openRuns()
    else if (id === "reports.delete") { if (cur) mode = "confirm" }
    else if (id === "reportsConfirm.yes") doDelete()
    else if (id === "reportsConfirm.no") mode = "list"
    else if (id === "reportsView.close") mode = "list"
    else if (id === "reportsView.down") scrollBy(40)
    else if (id === "reportsView.up") scrollBy(-40)
    else if (id === "reportsView.pagedown") scrollBy(flick.height - 40)
    else if (id === "reportsView.pageup") scrollBy(-(flick.height - 40))
    else if (id === "reportsEdit.cancel") { mode = "list"; form = null; formError = "" }
    else if (id === "reportsEdit.save") saveForm()
    else if (id === "reportsEdit.down") fieldIdx = Rep.moveIndex(fieldIdx, 1, fields.length)
    else if (id === "reportsEdit.up") fieldIdx = Rep.moveIndex(fieldIdx, -1, fields.length)
    else if (id === "reportsEdit.next") adjust(1)
    else if (id === "reportsEdit.prev") adjust(-1)
    else if (id === "reportsEdit.enter") {
      var key = fields[fieldIdx]
      if (Rep.TEXT_FIELDS.indexOf(key) >= 0) editText(key)
      else adjust(1)
    }
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
    objectName: "reportsKeys"
    anchors.fill: parent
    focus: true
    Keys.onPressed: function(e) {
      var id = Keymap.resolve([root.scope], Keymap.token(e))
      if (id === "") return
      e.accepted = true
      root.handle(id)
    }
  }

  ColumnLayout {
    anchors.fill: parent
    anchors.margins: Style.spacing.xxl
    spacing: Style.spacing.md

    RowLayout {
      Layout.fillWidth: true
      Text {
        text: root.mode === "edit" ? (root.form && root.form.id !== null ? "Edit report" : "New report")
              : root.mode === "preview" ? "Preview: " + (root.cur ? root.cur.name : "")
              : root.mode === "runs" ? "Runs: " + (root.cur ? root.cur.name : "") : "Reports"
        color: Commons.Color.menu.text
        font.family: Style.font.family
        font.pixelSize: Style.font.subtitle
        font.bold: true
      }
      Item { Layout.fillWidth: true }
      Text {
        text: root.loading ? "loading..." : Rep.mailStatusLine(root.mailStatus, root.mailError)
        color: root.mailStatus && root.mailStatus.authenticated ? Commons.Color.muted : Commons.Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    Text {
      Layout.fillWidth: true
      visible: text !== ""
      text: root.listError
      color: Commons.Color.urgent
      font.family: Style.font.family
      font.pixelSize: Style.font.body
      wrapMode: Text.WordWrap
    }
    Text {
      Layout.fillWidth: true
      visible: root.notice !== "" && root.mode !== "edit"
      text: root.notice
      color: root.noticeError ? Commons.Color.urgent : Commons.Color.accent
      font.family: Style.font.family
      font.pixelSize: Style.font.body
      wrapMode: Text.WordWrap
    }

    // ---- list
    ListView {
      id: list
      Layout.fillWidth: true
      Layout.fillHeight: true
      visible: root.mode === "list" || root.mode === "confirm"
      clip: true
      model: root.reports
      currentIndex: root.cursorIdx
      onCurrentIndexChanged: positionViewAtIndex(currentIndex, ListView.Contain)
      delegate: Rectangle {
        required property var modelData
        required property int index
        width: list.width
        height: col.implicitHeight + Style.spacing.md
        radius: Style.cornerRadius
        color: index === root.cursorIdx ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
        ColumnLayout {
          id: col
          anchors.left: parent.left; anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          anchors.margins: Style.spacing.md
          spacing: 2
          RowLayout {
            Text {
              text: (modelData.enabled ? "● " : "○ ") + modelData.name
              color: modelData.enabled ? Commons.Color.accent : Commons.Color.menu.text
              font.family: Style.font.family; font.pixelSize: Style.font.body; font.bold: true
            }
            Item { Layout.fillWidth: true }
            Text {
              text: Rep.lastRunText(modelData.last_run)
              color: modelData.last_run && modelData.last_run.status === "failed" ? Commons.Color.urgent : Commons.Color.muted
              font.family: Style.font.family; font.pixelSize: Style.font.bodySmall
            }
          }
          Text {
            Layout.fillWidth: true
            text: Rep.shapeLabel(modelData.shape) + "  ·  " + Rep.periodLabel(modelData.period) + "  ·  "
                  + Rep.scheduleLabel(modelData.schedule) + "  ·  next: " + Rep.nextRunText(modelData)
            color: Commons.Color.muted
            font.family: Style.font.family; font.pixelSize: Style.font.bodySmall
            elide: Text.ElideRight
          }
          Text {
            Layout.fillWidth: true
            visible: modelData.last_run && modelData.last_run.status === "failed" && !!modelData.last_run.detail
            text: modelData.last_run && modelData.last_run.detail ? modelData.last_run.detail : ""
            color: Commons.Color.urgent
            font.family: Style.font.family; font.pixelSize: Style.font.caption
            elide: Text.ElideRight
          }
        }
      }
      Text {
        anchors.centerIn: parent
        visible: root.reports.length === 0 && !root.loading && root.listError === ""
        text: "No reports yet. n creates one from the current query."
        color: Commons.Color.muted
        font.family: Style.font.family; font.pixelSize: Style.font.body
      }
    }

    Text {
      Layout.fillWidth: true
      visible: root.mode === "confirm"
      text: root.cur ? "Delete report '" + root.cur.name + "' and its timer?  y / n" : ""
      color: Commons.Color.urgent
      font.family: Style.font.family; font.pixelSize: Style.font.body; font.bold: true
    }

    // ---- editor
    ColumnLayout {
      Layout.fillWidth: true
      Layout.fillHeight: true
      visible: root.mode === "edit"
      spacing: Style.spacing.xs
      Repeater {
        model: root.fields
        delegate: RowLayout {
          required property string modelData
          required property int index
          Layout.fillWidth: true
          Text {
            Layout.preferredWidth: 130
            text: (index === root.fieldIdx ? "> " : "  ") + Rep.FIELD_LABELS[modelData]
            color: index === root.fieldIdx ? Commons.Color.accent : Commons.Color.muted
            font.family: Style.font.family; font.pixelSize: Style.font.body
          }
          Text {
            Layout.fillWidth: true
            text: root.form ? Rep.fieldValue(root.form, modelData) : ""
            color: Commons.Color.menu.text
            font.family: Style.font.family; font.pixelSize: Style.font.body
            font.bold: index === root.fieldIdx
            elide: Text.ElideRight
          }
        }
      }
      Text {
        Layout.fillWidth: true
        Layout.topMargin: Style.spacing.md
        text: root.nextError !== "" ? root.nextError : "Next runs: " + Rep.nextRunsText(root.nextRuns)
              + (root.nextOncal !== "" ? "   (" + root.nextOncal + ")" : "")
        color: root.nextError !== "" ? Commons.Color.urgent : Commons.Color.muted
        font.family: Style.font.family; font.pixelSize: Style.font.body
        wrapMode: Text.WordWrap
      }
      Text {
        Layout.fillWidth: true
        visible: root.formError !== ""
        text: root.formError
        color: Commons.Color.urgent
        font.family: Style.font.family; font.pixelSize: Style.font.body
        wrapMode: Text.WordWrap
      }
      Item { Layout.fillHeight: true }
    }

    // ---- preview and runs
    ColumnLayout {
      Layout.fillWidth: true
      Layout.fillHeight: true
      visible: root.mode === "preview" || root.mode === "runs"
      spacing: Style.spacing.sm
      Text {
        Layout.fillWidth: true
        visible: root.mode === "preview" && root.previewData !== null
        text: root.previewData ? (function() { var h = Rep.previewHeader(root.previewData)
                                  return h.subject + "\n" + h.period + "  ·  " + h.rows })() : ""
        color: Commons.Color.menu.text
        font.family: Style.font.family; font.pixelSize: Style.font.body; font.bold: true
        wrapMode: Text.WordWrap
      }
      Text {
        Layout.fillWidth: true
        visible: text !== ""
        text: root.mode === "preview" ? (root.previewError !== "" ? root.previewError : (root.busy ? "building preview..." : ""))
                                      : root.runsError
        color: root.previewError !== "" || root.runsError !== "" ? Commons.Color.urgent : Commons.Color.muted
        font.family: Style.font.family; font.pixelSize: Style.font.body
        wrapMode: Text.WordWrap
      }
      Flickable {
        id: flick
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true
        contentWidth: width
        contentHeight: bodyText.implicitHeight
        Text {
          id: bodyText
          width: flick.width
          text: root.mode === "preview" ? (root.previewData ? root.previewData.text : "")
                : root.runs.length === 0 && root.runsError === "" ? "No runs yet."
                : root.runs.map(Rep.runLine).join("\n")
          textFormat: Text.PlainText
          color: Commons.Color.menu.text
          font.family: "monospace"; font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WrapAnywhere
        }
      }
    }

    Text {
      Layout.fillWidth: true
      text: root.mode === "edit" ? "j/k field  Enter edit  h/l change  Space toggle  s save  Esc cancel"
            : root.mode === "preview" || root.mode === "runs" ? "j/k PgUp/PgDn scroll  Esc back"
            : root.mode === "confirm" ? "y delete  n cancel"
            : "n new  Enter edit  p preview  t send test  e enable/disable  r runs  d delete  Esc close"
      color: Commons.Color.muted
      font.family: Style.font.family; font.pixelSize: Style.font.caption
    }
  }
}
