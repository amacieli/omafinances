// Node tests for qml/state/Insight.js (Phase 5: dashboard, insights tabs, review queue, suggestions).
// Pure logic only; the overlays and AppShell wiring are checked with the scratch window.
// Run: node --test tests/qml/
const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const plain = x => JSON.parse(JSON.stringify(x))
const eq = (actual, expected) => assert.deepStrictEqual(plain(actual), expected)

// Loads every top-level function and the top-level constants of a qml/ .js file (no imports).
function load(file) {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', file), 'utf8')
  const names = [...src.matchAll(/^function (\w+)\(/gm)].map(m => m[1])
  const consts = [...src.matchAll(/^var (\w+) =/gm)].map(m => m[1])
  const ctx = {}
  vm.runInNewContext(src + '\n;__out = {' + names.concat(consts).join(',') + '};', ctx)
  return ctx.__out
}

const I = load('state/Insight.js')

test('clamp and moveIndex stay inside the list and never wrap', () => {
  eq(I.clamp(5, 3), 2)
  eq(I.clamp(-1, 3), 0)
  eq(I.clamp(4, 0), 0)
  eq(I.moveIndex(0, -1, 4), 0)
  eq(I.moveIndex(3, 1, 4), 3)
  eq(I.moveIndex(1, 1, 4), 2)
})

test('Tab cycles the four insight tabs and wraps both ways', () => {
  eq(I.TABS, ['dashboard', 'recurring', 'anomalies', 'transfers'])
  eq(I.nextTab(0, 1), 1)
  eq(I.nextTab(3, 1), 0)
  eq(I.nextTab(0, -1), 3)
  eq(I.nextTab(2, -1), 1)
  eq(I.tabTitle(2), 'Anomalies')
})

test('money: whole dollars, cents, signed, with thousands separators', () => {
  eq(I.signedMoneyCents(1234.5), '+$1,234.50')
  eq(I.signedMoneyCents(-200), '-$200.00')
  eq(I.signedMoneyCents(0), '$0.00')
  eq(I.money(18402.4), '$18,402')
  eq(I.money(-1234.5), '-$1,235')
  eq(I.money(0), '$0')
  eq(I.moneyCents(1234.5), '$1,234.50')
  eq(I.moneyCents(-42.1), '-$42.10')
  eq(I.signedMoney(1234), '+$1,234')
  eq(I.signedMoney(-200), '-$200')
  eq(I.signedMoney(0), '$0')
  eq(I.money(undefined), '$0')
})

test('percent and share: fractions, zero totals, NaN', () => {
  eq(I.pct(0.78), '78%')
  eq(I.pct(0.705), '71%')
  eq(I.pct(1), '100%')
  eq(I.pct(NaN), '0%')
  eq(I.share(25, 100), 0.25)
  eq(I.share(25, 0), 0)
  eq(I.share(150, 100), 1)
})

test('barWidth scales against max, keeps positive values visible, zero stays zero', () => {
  eq(I.barWidth(50, 100, 200), 100)
  eq(I.barWidth(100, 100, 200), 200)
  eq(I.barWidth(1, 100000, 200), 1)
  eq(I.barWidth(0, 100, 200), 0)
  eq(I.barWidth(-5, 100, 200), 0)
  eq(I.barWidth(5, 0, 200), 0)
  eq(I.barWidth(500, 100, 200), 200)
})

test('monthLabel and the dashboard range', () => {
  eq(I.monthLabel('2026-10'), 'Oct 2026')
  eq(I.monthLabel('2026-01'), 'Jan 2026')
  eq(I.monthLabel('bogus'), 'bogus')
  eq(I.rangeText({ from: '2025-12-01', to: '2026-11-01' }), '2025-12-01 to 2026-11-01')
  eq(I.rangeText(null), '')
})

test('category bars: width against the largest, share of total out, the Other row kept', () => {
  const bars = I.categoryBars([
    { category_id: 2, path: 'Food › Groceries', out: 800, count: 12 },
    { category_id: null, path: 'Uncategorized', out: 200, count: 3 },
    { category_id: null, path: 'Other', out: 100, count: 9 },
  ], 1000, 200)
  eq(bars.map(b => b.width), [200, 50, 25])
  eq(bars.map(b => b.shareText), ['80%', '20%', '10%'])
  eq(bars[0].label, '$800  \u00b7  80%  \u00b7  12 txns')
  eq(I.categoryBars([], 0, 200), [])
  eq(I.categoryBars([{ path: 'x', out: 0, count: 0 }], 0, 200)[0].shareText, '0%')
})

test('transfers note: excluded counts and the included state', () => {
  eq(I.transfersNote({ pairs: 2, total: 1000 }, false),
     'Transfers excluded: 2 pairs, $1,000 moved between your accounts. t includes them.')
  eq(I.transfersNote({ pairs: 1, total: 500 }, false),
     'Transfers excluded: 1 pair, $500 moved between your accounts. t includes them.')
  eq(I.transfersNote({ pairs: 0, total: 0 }, false),
     'No transfer pairs found, so nothing is excluded. t includes transfers when there are some.')
  eq(I.transfersNote({ pairs: 2, total: 1000 }, true),
     'Transfers included (press t to exclude them again).')
})

test('monthNet signs the net of a month', () => {
  eq(I.monthNet({ net: 1200 }), '+$1,200')
  eq(I.monthNet({ net: -340.2 }), '-$340')
})

test('recurring: annual cost by period, series line, active total', () => {
  const monthly = { period: 'monthly', count: 14, amount_median: 15.49, next_expected: '2026-12-03', active: true }
  const weekly = { period: 'weekly', count: 20, amount_median: 10, active: false }
  const yearly = { period: 'yearly', count: 3, amount_median: 120, active: true }
  eq(I.annualCost(monthly), 15.49 * 12)
  eq(I.annualCost(weekly), 520)
  eq(I.annualCost({ period: 'odd', amount_median: 9 }), 0)
  eq(I.seriesLine(monthly), 'monthly  \u00b7  14 charges  \u00b7  median $15.49  \u00b7  $186/yr  \u00b7  next 2026-12-03')
  eq(I.seriesLine({ period: 'odd', count: 3, amount_median: 1 }), 'odd  \u00b7  3 charges  \u00b7  median $1.00')
  eq(I.activeAnnualTotal([monthly, weekly, yearly]), 15.49 * 12 + 120)
  eq(I.recurringHeader([monthly, weekly, yearly]), '2 active of 3 series  \u00b7  $306/yr from active series')
  eq(I.recurringHeader([]), 'No recurring series yet: a series needs 3 or more charges on a steady schedule.')
})

test('anomalies: labels, header, and the list line', () => {
  eq(I.anomalyLabel('amount_spike'), 'spike')
  eq(I.anomalyLabel('duplicate'), 'duplicate')
  eq(I.anomalyLabel('new_payee'), 'new payee')
  eq(I.anomalyLabel('x'), 'x')
  eq(I.anomalyLine({ kind: 'amount_spike', date: '2026-11-02', payee: 'costco', amount: 900 }),
     'spike  \u00b7  2026-11-02  \u00b7  costco  \u00b7  $900.00')
  eq(I.anomaliesHeader([]), 'No anomalies: no spikes, duplicates or new payees in the data.')
  eq(I.anomaliesHeader([{}]), '1 item, newest first')
  eq(I.anomaliesHeader([{}, {}]), '2 items, newest first')
})

test('jump queries: payee quoting, exact ids, and per-tab selection', () => {
  eq(I.payeeQuery('netflix'), 'p:"netflix"')
  eq(I.payeeQuery('  whole  foods "x" '), 'p:"whole foods x"')
  eq(I.payeeQuery('   '), '')
  eq(I.payeeQuery(null), '')
  eq(I.idsQuery([10003, 10010]), 'id:10003,10010')
  eq(I.idsQuery([10003, null, 'x', -1, 2.5]), 'id:10003')
  eq(I.idsQuery([]), '')
  eq(I.jumpQueryFor('recurring', { payee: 'netflix' }), 'p:"netflix"')
  eq(I.jumpQueryFor('anomalies', { txn_id: 77, payee: 'trader joe\'s' }), 'id:77')
  eq(I.jumpQueryFor('anomalies', { payee: 'trader joe\'s' }), '')
  eq(I.jumpQueryFor('transfers', { out_id: 1, in_id: 2 }), 'id:1,2')
  eq(I.jumpQueryFor('dashboard', { payee: 'x' }), '')
})

test('transfers: pair line and header', () => {
  eq(I.pairLine({ out_account: 'Chase Checking', in_account: 'Ally Savings', amount: -500, days_apart: 1 }),
     'Chase Checking \u2192 Ally Savings   $500.00   1 day apart')
  eq(I.pairLine({ out_account: 'A', in_account: 'B', amount: 12.5, days_apart: 3 }),
     'A \u2192 B   $12.50   3 days apart')
  eq(I.transfersHeader({ count: 0, total: 0 }),
     'No transfer pairs: opposite amounts between two accounts within 4 days.')
  eq(I.transfersHeader({ count: 2, total: 1000 }), '2 pairs  \u00b7  $1,000 moved between accounts')
})

test('suggestions: top option, on/off reason, detail line', () => {
  const on = { enabled: true, reason: null, accuracy: { labeled: 412, folds: 5, top1: 0.6, top3: 0.78 } }
  const off = { enabled: false, reason: 'only 3 labeled rows (need 30)', accuracy: { labeled: 3, folds: 5, top1: 0, top3: 0 } }
  const sugg = { id: 9, options: [{ category_id: 2, path: 'Food › Groceries', confidence: 0.82, source: 'model' },
                                  { category_id: 4, path: 'Food › Coffee', confidence: 0.4, source: 'model' }] }
  eq(I.optionText(sugg.options[0]), 'Food › Groceries  82%  model')
  eq(I.topOption(sugg).category_id, 2)
  eq(I.topOption({ options: [] }), null)
  eq(I.topOption(null), null)
  eq(I.suggestionsEnabled(on), true)
  eq(I.suggestionsEnabled(off), false)
  eq(I.suggestionsEnabled(null), false)
  eq(I.offReason(off), 'only 3 labeled rows (need 30)')
  eq(I.offReason(on), '')
  eq(I.offReason(null), 'checking suggestions...')
  eq(I.offReason({ enabled: false, reason: null }), 'suggestions are off')
  eq(I.detailSuggestLine(on, sugg), 'Ctrl-y accepts: Food › Groceries  82%')
  eq(I.detailSuggestLine(on, { id: 9, options: [] }), 'no suggestion for this row (Ctrl-y needs one)')
  eq(I.detailSuggestLine(off, null), 'suggestions off: only 3 labeled rows (need 30)')
  eq(I.detailSuggestLine(null, null), 'checking suggestions...')
  eq(I.accuracyText(on.accuracy), 'top-3 accuracy 78% on 412 labeled')
})

test('review header: suggest mode and both bootstrap wordings', () => {
  const sugg = { enabled: true, reason: null, accuracy: { labeled: 412, folds: 5, top1: 0.6, top3: 0.78 } }
  eq(I.reviewHeader({ mode: 'suggest', acc: sugg, remaining: 412, index: 0, loaded: 50 }),
     { mode: 'Suggestions: top-3 accuracy 78% on 412 labeled', detail: '', progress: '412 groups left  \u00b7  group 1 of 412' })
  const boot = { enabled: false, reason: 'only 3 labeled rows (need 30)', accuracy: { labeled: 3, folds: 5, top1: 0, top3: 0 } }
  eq(I.reviewHeader({ mode: 'frequency', acc: boot, remaining: 88, index: 4, loaded: 50 }).mode,
     'Bootstrap: biggest payees first - suggestions unlock after 30 labeled rows and 70% top-3 accuracy (you have 3)')
  eq(I.reviewHeader({ mode: 'frequency', acc: boot, remaining: 88, index: 4, loaded: 50 }).detail, '')
  const lowAcc = { enabled: false, reason: 'top-3 accuracy 0.52 below 0.70',
                   accuracy: { labeled: 412, folds: 5, top1: 0.3, top3: 0.52 } }
  const h = I.reviewHeader({ mode: 'frequency', acc: lowAcc, remaining: 88, index: 0, loaded: 50 })
  eq(h.mode, 'Bootstrap: biggest payees first - suggestions unlock after 30 labeled rows and 70% top-3 accuracy (you have 412)')
  eq(h.detail, 'top-3 is 52% now')
  eq(I.reviewHeader({ mode: 'frequency', acc: null, remaining: 0, index: 0, loaded: 0 }),
     { mode: 'Bootstrap: biggest payees first', detail: 'checking suggestion accuracy...', progress: 'queue empty' })
  eq(I.reviewHeader({ mode: 'frequency', acc: boot, remaining: 1, index: 0, loaded: 1 }).progress,
     '1 group left  \u00b7  group 1 of 1')
})

test('option chips: at most three, numbered from 1, and the digit lookup', () => {
  const opts = [
    { category_id: 2, path: 'A › B', confidence: 0.9, source: 'rule' },
    { category_id: 4, path: 'C', confidence: 0.5, source: 'model' },
    { category_id: 5, path: 'D', confidence: 0.4, source: 'model' },
    { category_id: 6, path: 'E', confidence: 0.35, source: 'model' },
  ]
  const chips = I.optionChips(opts)
  eq(chips.map(c => c.n), [1, 2, 3])
  eq(chips[0].text, 'A › B   90%   rule')
  eq(I.optionFor(opts, '1').category_id, 2)
  eq(I.optionFor(opts, '3').category_id, 5)
  eq(I.optionFor(opts, '4'), null)
  eq(I.optionFor(opts, '0'), null)
  eq(I.optionFor([], '1'), null)
  eq(I.optionChips(null), [])
})

test('review queue state: paging, offsets, progress', () => {
  eq(I.reviewHasMore(50, 120, 50), true)
  eq(I.reviewHasMore(120, 120, 50), false)
  eq(I.reviewHasMore(50, 120, 20), false)
  eq(I.reviewNextOffset([{ id: 1 }, { id: 2 }]), 2)
  eq(I.reviewNextOffset([]), 0)
  eq(I.reviewProgress(100, 75), 0.25)
  eq(I.reviewProgress(0, 0), 1)
  eq(I.reviewProgress(10, 20), 0)
  eq(I.groupProgress(0, 0, 0), 'queue empty')
  eq(I.groupProgress(3, 1, 2), '3 groups left  \u00b7  group 2 of 3')
})

// A /review group row, as the Rust side will return it (group_* fields added by the lane in progress).
function grp(over) {
  return Object.assign({ id: 10, group_key: 'netflix', payee: 'NETFLIX', amount: -15.49, date: '2026-11-02',
    account: 'Chase', group_count: 41, group_total: -635.12, group_date_min: '2026-09-02',
    group_date_max: '2026-11-18', group_ids: [10, 11, 12, 13], options: [] }, over || {})
}

test('groups: key by payee and sign, ids as numbers, counts', () => {
  eq(I.gkey(grp()), 'netflix|in')
  eq(I.gkey(grp({ amount: 20 })), 'netflix|out')
  eq(I.groupIds(grp()), [10, 11, 12, 13])
  eq(I.groupIds(grp({ group_ids: null })), [10])
  eq(I.groupIds(grp({ group_ids: ['7', 'x', 8] })), [7, 8])
  eq(I.groupCount(grp()), 41)
  eq(I.groupCount(grp({ group_count: 0 })), 4)
  eq(I.groupPageIds(grp({ group_ids: Array.from({ length: 120 }, (_, i) => i + 1) }), 50).length, 50)
  eq(I.groupPageIds(grp({ group_ids: Array.from({ length: 120 }, (_, i) => i + 1) }), 100).length, 20)
})

test('ticks: default all ticked, toggle one row, all and none for a group', () => {
  const g = grp()
  const k = I.gkey(g)
  eq(I.tickedIds(g, {}), [10, 11, 12, 13])
  eq(I.willText(g, {}), 'Will categorize 4 of 41')
  let u = I.toggleTick({}, k, 11)
  eq(I.tickedIds(g, u), [10, 12, 13])
  eq(I.willText(g, u), 'Will categorize 3 of 41')
  eq(I.allTicked(g, u), false)
  u = I.toggleTick(u, k, 11)
  eq(u, {})
  eq(I.allTicked(g, u), true)
  u = I.setGroupTicks({}, k, g, false)
  eq(I.tickedCount(g, u), 0)
  eq(I.willText(g, u), 'Will categorize 0 of 41')
  eq(I.setGroupTicks(u, k, g, true), {})
  // ticks of one group never leak into another
  eq(I.tickedCount(grp({ group_key: 'hulu' }), I.setGroupTicks({}, k, g, false)), 4)
})

test('pending: default is the top option when suggestions are on, else none; apply plan', () => {
  const opts = [{ category_id: 3, path: 'Bills › Streaming' }, { category_id: 4, path: 'Other' }]
  const g = grp({ options: opts })
  eq(I.pendingFor({}, g, true), { category_id: 3, path: 'Bills › Streaming' })
  eq(I.pendingFor({}, g, false), null)
  eq(I.pendingFor({}, grp(), true), null)
  eq(I.pendingText(null), 'none: press 1/2/3 or c')
  eq(I.pendingText({ category_id: 4, path: 'Other' }), 'category: Other   (y applies)')
  const pend = { [I.gkey(g)]: { category_id: 4, path: 'Other' } }
  eq(I.pendingFor(pend, g, true), { category_id: 4, path: 'Other' })
  eq(I.applyPlan(g, { category_id: 4, path: 'Other' }, {}),
     { ok: true, ids: [10, 11, 12, 13], category_id: 4, path: 'Other' })
  eq(I.applyPlan(g, null, {}).ok, false)
  eq(I.applyPlan(g, null, {}).message, 'no category yet: press 1, 2 or 3, or c to pick one')
  const none = I.setGroupTicks({}, I.gkey(g), g, false)
  eq(I.applyPlan(g, { category_id: 4, path: 'Other' }, none).message,
     'no rows ticked: Space ticks a row, a ticks all')
  eq(I.applyPlan(g, { category_id: 4, path: 'Other' }, I.toggleTick({}, I.gkey(g), 12)).ids, [10, 11, 13])
})

test('group lines: header, date span, row line, chips from the group options', () => {
  eq(I.groupLine(grp()), 'NETFLIX  \u00b7  41 rows  \u00b7  -$635.12  \u00b7  2026-09-02 to 2026-11-18')
  eq(I.groupLine(grp({ group_count: 1, group_ids: [10], group_date_min: '2026-11-02', group_date_max: '2026-11-02' })),
     'NETFLIX  \u00b7  1 row  \u00b7  -$635.12  \u00b7  2026-11-02')
  eq(I.rowLine({ date: '2026-11-02', payee: 'AMAZON', amount: -24.99, account: 'Chase' }),
     '2026-11-02  AMAZON  -$24.99  Chase')
  eq(I.optionChips(grp({ options: [{ category_id: 3, path: 'Bills', confidence: 0.9, source: 'rule' }] }).options)
       .map(c => c.n), [1])
})

test('items: groups, expanded rows and "more", cursor after a group leaves', () => {
  const a = grp()
  const b = grp({ id: 20, group_key: 'hulu', payee: 'HULU', group_ids: [20, 21], group_count: 2 })
  const groups = [a, b]
  const ka = I.gkey(a)
  eq(I.groupItems(groups, {}, {}, {}).map(i => i.key), ['g:netflix|in', 'g:hulu|in'])
  const rowsA = [{ id: 10, payee: 'NETFLIX' }, { id: 11, payee: 'NETFLIX' }]
  const items = I.groupItems(groups, { [ka]: true }, { [ka]: rowsA }, {})
  eq(items.map(i => i.key),
     ['g:netflix|in', 'r:netflix|in:10', 'r:netflix|in:11', 'm:netflix|in', 'g:hulu|in'])
  eq(items[3].left, 2)
  eq(I.groupItems(groups, { [ka]: true }, {}, { [ka]: true })[1].loading, true)
  // the cursor on the removed group's header lands on the next group's header
  const after = I.groupItems([b], {}, {}, {})
  eq(I.cursorIndex(after, 'g:netflix|in', 0), 0)
  eq(after[I.cursorIndex(after, 'g:netflix|in', 0)].key, 'g:hulu|in')
  // a row that vanished falls back to a header near its position
  eq(I.cursorIndex(items, 'r:netflix|in:99', 1), 0)
  eq(I.cursorIndex(items, 'm:gone|in', 3), 4)
  eq(I.cursorIndex(items, 'g:hulu|in', 0), 4)
  eq(I.cursorIndex([], 'g:x', 0), 0)
  eq(I.groupStep(items, 0, 1), 4)
  eq(I.groupStep(items, 4, 1), 4)
  eq(I.groupStep(items, 2, -1), 0)
  eq(I.groupStep(items, 0, -1), 0)
  eq(I.removeGroup(groups, ka).map(I.gkey), ['hulu|in'])
  eq(I.groupIndexOf(groups, 'hulu|in'), 1)
  eq(I.groupIndexOf(groups, 'x|in'), -1)
  eq(I.pruneTo({ [ka]: 1, 'x|in': 2 }, [ka]), { [ka]: 1 })
  eq(I.anyExpanded({ [ka]: true }), true)
  eq(I.anyExpanded({}), false)
})

test('dashboard title joins the range', () => {
  eq(I.dashboardTitle({ from: '2025-12-01', to: '2026-11-01' }), 'Dashboard  \u00b7  2025-12-01 to 2026-11-01')
  eq(I.dashboardTitle(null), 'Dashboard')
})
