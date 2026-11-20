// Node tests for qml/state/Reports.js (Phase 6 reports overlay). Run: node --test tests/qml/
const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const plain = x => JSON.parse(JSON.stringify(x))
const eq = (a, b) => assert.deepStrictEqual(plain(a), b)

function load(file) {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', file), 'utf8')
  const names = [...src.matchAll(/^function (\w+)\(/gm)].map(m => m[1])
  const consts = [...src.matchAll(/^var (\w+) =/gm)].map(m => m[1])
  const ctx = {}
  vm.runInNewContext(src + '\n;__out = {' + names.concat(consts).join(',') + '};', ctx)
  return ctx.__out
}
const R = load('state/Reports.js')

test('schedule labels', () => {
  assert.equal(R.scheduleLabel({ kind: 'daily', time: '07:30' }), 'daily 07:30')
  assert.equal(R.scheduleLabel({ kind: 'weekly', weekday: 'mon', time: '08:00' }), 'Mondays 08:00')
  assert.equal(R.scheduleLabel({ kind: 'monthly', day: 1, time: '07:30' }), 'monthly on 1st 07:30')
  assert.equal(R.scheduleLabel({ kind: 'monthly', day: 'last', time: '07:30' }), 'monthly on the last day 07:30')
  assert.equal(R.scheduleLabel({ kind: 'monthly', day: 22, time: '07:30' }), 'monthly on 22nd 07:30')
  assert.equal(R.scheduleLabel({ kind: 'monthly', day: 12, time: '07:30' }), 'monthly on 12th 07:30')
  assert.match(R.scheduleLabel({ kind: 'quarterly', day: 3, time: '07:30' }), /quarterly on 3rd 07:30 \(Jan Apr Jul Oct\)/)
  assert.equal(R.scheduleLabel(null), 'no schedule')
})

test('changing the preset keeps what still applies', () => {
  eq(R.withKind({ kind: 'weekly', weekday: 'fri', time: '09:15' }, 'daily'), { kind: 'daily', time: '09:15' })
  eq(R.withKind({ kind: 'monthly', day: 'last', time: '09:15' }, 'quarterly'), { kind: 'quarterly', day: 'last', time: '09:15' })
  eq(R.withKind({ kind: 'daily', time: '09:15' }, 'weekly'), { kind: 'weekly', weekday: 'mon', time: '09:15' })
})

test('schedule preview path', () => {
  assert.equal(R.schedulePreviewPath({ kind: 'weekly', weekday: 'mon', time: '07:30' }), '/schedule/preview?kind=weekly&time=07%3A30&weekday=mon')
  assert.equal(R.schedulePreviewPath({ kind: 'monthly', day: 'last', time: '07:30' }), '/schedule/preview?kind=monthly&time=07%3A30&day=last')
  assert.equal(R.schedulePreviewPath({ kind: 'daily', time: '07:30' }), '/schedule/preview?kind=daily&time=07%3A30')
})

test('validation mirrors the backend rules', () => {
  const f = R.newForm('p:amazon', 'me@x.com', 'Amazon')
  assert.equal(R.validateForm(f), '')
  assert.equal(R.validateForm(Object.assign({}, f, { name: '  ' })), 'name is required')
  assert.match(R.validateForm(Object.assign({}, f, { schedule: { kind: 'daily', time: '7:30' } })), /HH:MM/)
  assert.match(R.validateForm(Object.assign({}, f, { schedule: { kind: 'monthly', day: 29, time: '07:30' } })), /1 to 28/)
  assert.equal(R.validateForm(Object.assign({}, f, { schedule: { kind: 'monthly', day: 'last', time: '23:59' } })), '')
  assert.match(R.validateForm(Object.assign({}, f, { recipients: ['nope'] })), /not an email/)
  assert.match(R.validateForm(Object.assign({}, f, { shape: 'pie' })), /unknown shape/)
  assert.match(R.validateForm(Object.assign({}, f, { period: 'x' })), /unknown period/)
})

test('newForm defaults: current query, sender as recipient, redact off', () => {
  const f = R.newForm('c:food', 'me@x.com', '')
  assert.equal(f.query, 'c:food'); eq(f.recipients, ['me@x.com']); assert.equal(f.redact, false)
  eq(R.newForm('', null).recipients, [])
})

test('formBody normalizes the schedule and trims', () => {
  const f = Object.assign(R.newForm(' q ', 'a@b.co', ' N '), { schedule: { kind: 'monthly', day: '5', time: '07:30', weekday: 'mon' } })
  eq(R.formBody(f), { name: 'N', query: 'q', shape: 'by_category', period: 'last_full_month',
    schedule: { kind: 'monthly', time: '07:30', day: 5 }, recipients: ['a@b.co'], redact: false })
})

test('edit fields depend on the schedule kind', () => {
  const f = R.newForm('', 'a@b.co')
  assert.ok(R.editFields(f).includes('day') && !R.editFields(f).includes('weekday'))
  const w = R.adjustField(f, 'kind', -1) // monthly -> weekly
  assert.equal(w.schedule.kind, 'weekly')
  assert.ok(R.editFields(w).includes('weekday') && !R.editFields(w).includes('day'))
  assert.ok(!R.editFields(R.adjustField(w, 'kind', -1)).includes('weekday'))
})

test('adjustField cycles, wraps and toggles without mutating', () => {
  const f = R.newForm('', 'a@b.co')
  assert.equal(R.adjustField(f, 'shape', 1).shape, 'by_payee')
  assert.equal(R.adjustField(R.adjustField(f, 'shape', 1), 'shape', -1).shape, 'by_category')
  assert.equal(R.adjustField(Object.assign({}, f, { shape: 'table' }), 'shape', -1).shape, 'summary')
  assert.equal(R.adjustField(f, 'redact', 1).redact, true)
  assert.equal(f.redact, false)
  assert.equal(R.adjustField(f, 'day', -1).schedule.day, 'last')
  assert.equal(R.adjustField({ schedule: { kind: 'monthly', day: 'last', time: '07:30' } }, 'day', 1).schedule.day, 1)
})

test('next runs and run history formatting', () => {
  assert.equal(R.formatNext('2026-11-23T07:30:00'), 'Mon 2026-11-23 07:30')
  assert.equal(R.nextRunsText(['2026-12-01T07:30:00', '2027-01-01T07:30:00']), 'Tue 2026-12-01 07:30  ·  Fri 2027-01-01 07:30')
  assert.equal(R.nextRunsText([]), 'none')
  assert.equal(R.lastRunText(null), 'never run')
  assert.equal(R.lastRunText({ status: 'ok', finished_at: '2026-11-01T07:30:12' }), 'ok 2026-11-01 07:30')
  assert.equal(R.nextRunText({ enabled: false, next_runs: ['2026-12-01T07:30:00'] }), 'disabled')
  assert.equal(R.nextRunText({ enabled: true, next_runs: ['2026-12-01T07:30:00'] }), 'Tue 2026-12-01 07:30')
  assert.equal(R.runLine({ started_at: '2026-11-01T07:30:00', finished_at: '2026-11-01T07:30:05', status: 'failed', trigger: 'timer', rows: null, detail: 'boom' }),
    '2026-11-01 07:30 failed  timer              boom')
  assert.match(R.runLine({ started_at: '2026-11-01T07:30:00', status: 'ok', trigger: 'test', rows: 42 }), /42 rows/)
})

test('mail status line', () => {
  assert.equal(R.mailStatusLine({ authenticated: true, sender: 'me@x.com', client_configured: true }), 'Sending as me@x.com')
  assert.equal(R.mailStatusLine({ authenticated: false, sender: null, client_configured: true }), 'Gmail not authorized: run omafinances gmail-auth')
  assert.equal(R.mailStatusLine({ authenticated: false, sender: null, client_configured: false }), 'client not configured: set [mail] client_secret_file')
  assert.equal(R.mailStatusLine(null, 'HTTP 500'), 'HTTP 500')
})

test('error envelopes are verbatim; 409 not_authenticated adds the hint', () => {
  assert.equal(R.envelopeText(400, { error: 'bad_query', message: 'unknown field zz' }), 'bad_query: unknown field zz')
  const t = R.envelopeText(409, { error: 'not_authenticated', message: 'Gmail is not authorized' })
  assert.ok(t.startsWith('not_authenticated: Gmail is not authorized'))
  assert.match(t, /omafinances gmail-auth/)
  assert.equal(R.envelopeText(500, null), 'HTTP 500')
  assert.ok(!/gmail-auth/.test(R.envelopeText(409, { error: 'duplicate', message: 'name taken' })))
})

test('list cursor and preview scroll clamp', () => {
  assert.equal(R.moveIndex(0, -1, 3), 0); assert.equal(R.moveIndex(2, 1, 3), 2); assert.equal(R.moveIndex(0, 1, 0), 0)
  assert.equal(R.scrollBy(0, 5, 100, 20), 5); assert.equal(R.scrollBy(78, 5, 100, 20), 80); assert.equal(R.scrollBy(2, -9, 100, 20), 0)
  assert.equal(R.scrollBy(0, 5, 10, 20), 0)
})

test('preview header', () => {
  eq(R.previewHeader({ subject: '[OmaFinances] N: Oct', rows: 12, period: { label: 'October', from: '2026-10-01', to: '2026-10-31' } }),
    { subject: '[OmaFinances] N: Oct', period: 'October (2026-10-01 to 2026-10-31)', rows: '12 rows' })
})

test('recipients parse', () => {
  eq(R.parseRecipients('a@b.co, c@d.co;  e@f.co'), ['a@b.co', 'c@d.co', 'e@f.co'])
})
