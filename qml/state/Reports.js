// Phase 6 reports: pure logic for the Reports overlay (P). Schedule presets and labels, the form and
// its validation (a mirror of the backend rules; the backend stays the authority and its error
// envelope is shown verbatim), next-run and run-history formatting, the list cursor.
// Checked with node: tests/qml/reports.test.js.

var SHAPES = ["table", "by_category", "by_payee", "by_month", "summary"]
var SHAPE_LABELS = { table: "table", by_category: "by category", by_payee: "by payee", by_month: "by month", summary: "summary" }
var PERIODS = ["last_7d", "last_full_week", "last_full_month", "month_to_date", "last_full_quarter", "ytd", "all"]
var PERIOD_LABELS = {
  last_7d: "last 7 days", last_full_week: "last full week (Mon-Sun)", last_full_month: "last full month",
  month_to_date: "month to date", last_full_quarter: "last full quarter", ytd: "year to date", all: "all time"
}
var KINDS = ["daily", "weekly", "monthly", "quarterly"]
var WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
var WEEKDAY_NAMES = { mon: "Monday", tue: "Tuesday", wed: "Wednesday", thu: "Thursday", fri: "Friday", sat: "Saturday", sun: "Sunday" }
var DAYS = []
for (var di = 1; di <= 28; di++) DAYS.push(di)
DAYS.push("last")

var NOT_AUTH_HINT = "Run `omafinances gmail-auth` in a terminal, then try again."

function clamp(i, n) { return n <= 0 ? 0 : Math.max(0, Math.min(n - 1, i)) }
function moveIndex(i, dir, n) { return clamp(i + dir, n) }

// The next/previous value of a list (wraps: these are choices, not a cursor).
function cycle(list, cur, dir) {
  var i = list.indexOf(cur)
  if (i < 0) i = 0
  return list[(i + dir + list.length * 4) % list.length]
}

function shapeLabel(s) { return SHAPE_LABELS[s] || String(s) }
function periodLabel(p) { return PERIOD_LABELS[p] || String(p) }

function pad2(n) { return (n < 10 ? "0" : "") + n }

function ordinal(d) {
  if (d === "last") return "the last day"
  var n = Number(d), t = n % 100
  var s = (t >= 11 && t <= 13) ? "th" : ({ 1: "st", 2: "nd", 3: "rd" })[n % 10] || "th"
  return n + s
}

// "daily 07:30", "Mondays 07:30", "monthly on the 1st 07:30", "quarterly on the 1st 07:30 (Jan Apr Jul Oct)".
function scheduleLabel(s) {
  if (!s || !s.kind) return "no schedule"
  var t = s.time || "07:30"
  if (s.kind === "daily") return "daily " + t
  if (s.kind === "weekly") return (WEEKDAY_NAMES[s.weekday] || String(s.weekday)) + "s " + t
  if (s.kind === "monthly") return "monthly on " + ordinal(s.day) + " " + t
  if (s.kind === "quarterly") return "quarterly on " + ordinal(s.day) + " " + t + " (Jan Apr Jul Oct)"
  return String(s.kind)
}

function defaultSchedule(kind) {
  if (kind === "weekly") return { kind: "weekly", weekday: "mon", time: "07:30" }
  if (kind === "monthly") return { kind: "monthly", day: 1, time: "07:30" }
  if (kind === "quarterly") return { kind: "quarterly", day: 1, time: "07:30" }
  return { kind: "daily", time: "07:30" }
}

// Changing the preset keeps the time, weekday and day the user already chose where they still apply.
function withKind(s, kind) {
  var d = defaultSchedule(kind)
  if (s && s.time) d.time = s.time
  if (s && s.weekday && kind === "weekly") d.weekday = s.weekday
  if (s && s.day !== undefined && (kind === "monthly" || kind === "quarterly")) d.day = s.day
  return d
}

function normalizeSchedule(s) {
  var o = { kind: s.kind, time: s.time }
  if (s.kind === "weekly") o.weekday = s.weekday
  if (s.kind === "monthly" || s.kind === "quarterly") o.day = s.day === "last" ? "last" : Number(s.day)
  return o
}

// Query string of GET /schedule/preview for a schedule.
function schedulePreviewPath(s) {
  var p = "/schedule/preview?kind=" + encodeURIComponent(s.kind) + "&time=" + encodeURIComponent(s.time || "")
  if (s.kind === "weekly") p += "&weekday=" + encodeURIComponent(s.weekday)
  if (s.kind === "monthly" || s.kind === "quarterly") p += "&day=" + encodeURIComponent(String(s.day))
  return p
}

function validTime(t) { return /^([01]\d|2[0-3]):[0-5]\d$/.test(String(t)) }

function timeError(t) { return validTime(t) ? "" : "time must be HH:MM, 24 hour (for example 07:30)" }

function parseRecipients(text) {
  return String(text || "").split(/[,;\s]+/).filter(function(x) { return x !== "" })
}

function recipientsError(list) {
  for (var i = 0; i < list.length; i++) {
    if (!/^[^@\s<>,;"]+@[^@\s<>,;"]+\.[^@\s<>,;"]+$/.test(list[i])) return "not an email address: " + list[i]
  }
  return ""
}

// A new form. query = the current table query; sender = the authenticated sender (default recipient).
function newForm(query, sender, name) {
  return {
    id: null, name: name || "", query: query || "", shape: "by_category", period: "last_full_month",
    schedule: defaultSchedule("monthly"), recipients: sender ? [sender] : [], redact: false
  }
}

function formFromReport(r) {
  return {
    id: r.id, name: r.name, query: r.query || "", shape: r.shape, period: r.period,
    schedule: Object.assign({}, r.schedule), recipients: (r.recipients || []).slice(), redact: r.redact === true || r.redact === 1
  }
}

// "" when valid, else the first problem (the same rules the backend applies; it says the rest).
function validateForm(f) {
  if (String(f.name || "").trim() === "") return "name is required"
  if (String(f.name).length > 80) return "name is too long (80 characters at most)"
  if (SHAPES.indexOf(f.shape) < 0) return "unknown shape: " + f.shape
  if (PERIODS.indexOf(f.period) < 0) return "unknown period: " + f.period
  var s = f.schedule || {}
  if (KINDS.indexOf(s.kind) < 0) return "unknown schedule: " + s.kind
  var te = timeError(s.time)
  if (te) return te
  if (s.kind === "weekly" && WEEKDAYS.indexOf(s.weekday) < 0) return "unknown weekday: " + s.weekday
  if ((s.kind === "monthly" || s.kind === "quarterly") && DAYS.indexOf(s.day === "last" ? "last" : Number(s.day)) < 0)
    return "day must be 1 to 28 or last"
  return recipientsError(f.recipients || [])
}

// Body for POST /reports and PUT /reports/{id}.
function formBody(f) {
  return {
    name: String(f.name).trim(), query: String(f.query || "").trim(), shape: f.shape, period: f.period,
    schedule: normalizeSchedule(f.schedule), recipients: f.recipients || [], redact: f.redact === true
  }
}

// Editor rows in order; weekday and day show only for the schedules that use them.
function editFields(f) {
  var k = f.schedule ? f.schedule.kind : "daily"
  var out = ["name", "query", "shape", "period", "kind"]
  if (k === "weekly") out.push("weekday")
  if (k === "monthly" || k === "quarterly") out.push("day")
  out.push("time", "recipients", "redact")
  return out
}

var FIELD_LABELS = {
  name: "Name", query: "Query", shape: "Shape", period: "Period", kind: "Schedule", weekday: "Weekday",
  day: "Day of month", time: "Time", recipients: "Recipients", redact: "Redact"
}
var TEXT_FIELDS = ["name", "query", "time", "recipients"]

function fieldValue(f, key) {
  var s = f.schedule || {}
  if (key === "name") return f.name
  if (key === "query") return f.query === "" ? "(all transactions)" : f.query
  if (key === "shape") return shapeLabel(f.shape)
  if (key === "period") return periodLabel(f.period)
  if (key === "kind") return s.kind
  if (key === "weekday") return WEEKDAY_NAMES[s.weekday] || String(s.weekday)
  if (key === "day") return s.day === "last" ? "last day" : String(s.day)
  if (key === "time") return s.time
  if (key === "recipients") return (f.recipients || []).join(", ")
  if (key === "redact") return f.redact ? "on (account names hidden, payees cut to 12 characters, notes hidden)" : "off"
  return ""
}

// Changes a cycling or toggle field by dir (+1/-1). Returns a new form.
function adjustField(f, key, dir) {
  var g = Object.assign({}, f, { schedule: Object.assign({}, f.schedule) })
  if (key === "shape") g.shape = cycle(SHAPES, f.shape, dir)
  else if (key === "period") g.period = cycle(PERIODS, f.period, dir)
  else if (key === "kind") g.schedule = withKind(f.schedule, cycle(KINDS, f.schedule.kind, dir))
  else if (key === "weekday") g.schedule.weekday = cycle(WEEKDAYS, f.schedule.weekday, dir)
  else if (key === "day") g.schedule.day = cycle(DAYS, f.schedule.day === "last" ? "last" : Number(f.schedule.day), dir)
  else if (key === "redact") g.redact = !f.redact
  return g
}

// "Mon 2026-11-23 07:30" from an ISO local time ("2026-11-23T07:30:00" or with a space).
function formatNext(iso) {
  var m = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/.exec(String(iso || ""))
  if (!m) return String(iso || "")
  var d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]))
  var dn = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][d.getUTCDay()]
  return dn + " " + m[1] + "-" + m[2] + "-" + m[3] + " " + m[4] + ":" + m[5]
}

function nextRunsText(list) {
  if (!list || list.length === 0) return "none"
  return list.map(formatNext).join("  ·  ")
}

function shortTime(iso) {
  var m = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2})/.exec(String(iso || ""))
  return m ? m[1] + " " + m[2] : String(iso || "")
}

// The list column for the last run: "ok 2026-11-01 07:30", "failed 2026-11-01 07:30", or "never run".
function lastRunText(lr) {
  if (!lr) return "never run"
  return String(lr.status) + (lr.finished_at ? " " + shortTime(lr.finished_at) : "")
}

function nextRunText(r) {
  if (!r.enabled) return "disabled"
  return r.next_runs && r.next_runs.length > 0 ? formatNext(r.next_runs[0]) : "none"
}

// One run-history line: "2026-11-01 07:30  ok      timer   42 rows  detail".
function runLine(run) {
  var pad = function(s, n) { s = String(s); while (s.length < n) s += " "; return s }
  var parts = [shortTime(run.finished_at || run.started_at), pad(run.status, 7), pad(run.trigger, 8),
               run.rows !== null && run.rows !== undefined ? pad(run.rows + " rows", 9) : pad("", 9)]
  return parts.join(" ") + (run.detail ? " " + run.detail : "")
}

function mailStatusLine(st, err) {
  if (err) return err
  if (!st) return "mail status: loading..."
  if (st.authenticated && st.sender) return "Sending as " + st.sender
  if (st.client_configured === false) return "client not configured: set [mail] client_secret_file"
  return "Gmail not authorized: run omafinances gmail-auth"
}

// A backend error envelope {error, message}: shown verbatim. 409 not_authenticated gets the hint.
function envelopeText(status, body) {
  var j = body
  var text = j && (j.error || j.message) ? (j.error ? String(j.error) + ": " : "") + String(j.message || "") : "HTTP " + status
  if (status === 409 && j && j.error === "not_authenticated") text += "\n" + NOT_AUTH_HINT
  return text
}

// Preview header: "Subject  ·  period label (from..to)  ·  N rows".
function previewHeader(p) {
  var per = p.period ? p.period.label + " (" + p.period.from + " to " + p.period.to + ")" : ""
  return { subject: p.subject || "", period: per, rows: (p.rows !== undefined ? p.rows : 0) + " rows" }
}

function textLines(text) { return String(text || "").split("\n") }

// Scroll offset of the text part, clamped to [0, max(0, total - visible)].
function scrollBy(off, delta, total, visible) { return Math.max(0, Math.min(Math.max(0, total - visible), off + delta)) }
