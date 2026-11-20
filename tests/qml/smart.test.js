// Node tests for qml/state/Smart.js (categorize with preview). Pure logic only; the overlay and
// AppShell wiring are checked by the scratch-window harness. Run: node --test tests/qml/
const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const plain = x => JSON.parse(JSON.stringify(x))
const eq = (actual, expected) => assert.deepStrictEqual(plain(actual), expected)

// Loads every top-level function of a qml/ .js file into a fresh sandbox (the file has no imports).
function load(file) {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', file), 'utf8')
  const names = [...src.matchAll(/^function (\w+)\(/gm)].map(m => m[1])
  const ctx = {}
  vm.runInNewContext(src + '\n;__out = {' + names.join(',') + '};', ctx)
  return ctx.__out
}

const S = load('state/Smart.js')

// Fixture: a seed cluster, an exact match, a fuzzy match, a conflict, a same-category cluster,
// and a 60-row cluster for paging.
const ex = (id, payee, amount, cat) => ({ id, date: '2026-11-0' + (id % 9 + 1), payee, amount, account: 'Chase', category_path: cat || null, why: 'earliest' })
const big = []
for (let i = 1; i <= 60; i++) big.push(1000 + i)
const CLUSTERS = [
  { id: 'c1', label: 'amazon', tier: 'seed', confidence: 1, reason: 'same normalized payee', count: 3, total: 120.5,
    date_min: '2026-10-01', date_max: '2026-11-02', amount_min: 10, amount_max: 80.25, status: 'uncategorized',
    conflict_categories: [], default_selected: true,
    exemplars: [ex(1, 'AMAZON', 10), ex(2, 'AMAZON', 80.25), ex(3, 'AMAZON', 30)], txn_ids: [3, 2, 1] },
  { id: 'c2', label: 'amzn mktp', tier: 'fuzzy', confidence: 0.86, reason: "payee 0.86 similar to 'amazon'", count: 2, total: 40,
    date_min: '2026-10-05', date_max: '2026-10-20', amount_min: 15, amount_max: 25, status: 'uncategorized',
    conflict_categories: [], default_selected: true,
    exemplars: [ex(4, 'AMZN MKTP', 15), ex(5, 'AMZN MKTP', 25)], txn_ids: [5, 4] },
  { id: 'c3', label: 'hardware depot', tier: 'exact', confidence: 1, reason: 'same normalized payee', count: 2, total: 90,
    date_min: '2026-09-01', date_max: '2026-09-15', amount_min: 40, amount_max: 50, status: 'conflict',
    conflict_categories: ['Groceries'], default_selected: false,
    exemplars: [ex(6, 'HARDWARE DEPOT', 40, 'Groceries'), ex(7, 'HARDWARE DEPOT', 50, 'Groceries')], txn_ids: [7, 6] },
  { id: 'c4', label: 'hardware depot', tier: 'recurring', confidence: 0.9, reason: 'same recurring id', count: 1, total: 12,
    date_min: '2026-08-01', date_max: '2026-08-01', amount_min: 12, amount_max: 12, status: 'same',
    conflict_categories: [], default_selected: false,
    exemplars: [ex(8, 'HARDWARE DEPOT', 12, 'Maintenance')], txn_ids: [8] },
  { id: 'c5', label: 'lawn care co', tier: 'exact', confidence: 1, reason: 'same normalized payee', count: 60, total: 3000,
    date_min: '2026-01-01', date_max: '2026-11-01', amount_min: 50, amount_max: 50, status: 'uncategorized',
    conflict_categories: [], default_selected: true,
    exemplars: [ex(9, 'LAWN CARE', 50), ex(10, 'LAWN CARE', 50)], txn_ids: big }
]
const KEYS = S.clusterKeys(CLUSTERS)

test('clusterKeys uses the label and numbers repeats in order', () => {
  eq(KEYS, ['amazon', 'amzn mktp', 'hardware depot', 'hardware depot#2', 'lawn care co'])
})

test('threshold steps by 0.05, clamps to 0.5..1.0 and stays on hundredths', () => {
  eq(S.stepThreshold(0.75, 1), 0.8)
  eq(S.stepThreshold(0.75, -1), 0.7)
  let t = 0.75
  for (let i = 0; i < 10; i++) t = S.stepThreshold(t, 1)
  eq(t, 1)
  for (let i = 0; i < 30; i++) t = S.stepThreshold(t, -1)
  eq(t, 0.5)
  eq(S.stepThreshold(0.5, -1), 0.5)
  eq(S.stepThreshold(0.7 + 0.05, 0), 0.75)
  eq(S.clampThreshold(NaN), 0.75)
  eq(S.thresholdPercent(0.75), '75%')
  eq(S.thresholdPercent(0.85), '85%')
  eq(S.thresholdPercent(1), '100%')
})

test('money formats: whole dollars for the header, cents for rows, sign kept', () => {
  eq(S.money(18402.4), '$18,402')
  eq(S.money(-1234), '-$1,234')
  eq(S.money(0), '$0')
  eq(S.moneyCents(1234.5), '$1,234.50')
  eq(S.moneyCents(-3), '-$3.00')
})

test('header matches the contract wording and counts applied rows only', () => {
  const summary = { uncategorized_total: 1194 }
  // Only the seed and fuzzy clusters selected: 3 + 2 rows, 120.5 + 40 dollars.
  const sel = { 'amazon': true, 'amzn mktp': true, 'hardware depot': false, 'hardware depot#2': false, 'lawn care co': false }
  const t = S.tally(CLUSTERS, sel, false)
  eq(t.applyRows, 5)
  eq(S.headerText(summary, t, false), 'Will categorize 5 of 1,194 uncategorized  \u00b7  2 conflicts skipped  \u00b7  $161')
})

test('conflicts: skipped without overwrite whether selected or not; overwritten with it', () => {
  const sel = Object.assign(S.setAll(CLUSTERS, false), { 'hardware depot': true })
  const off = S.tally(CLUSTERS, sel, false)
  eq([off.applyRows, off.conflictRows, off.conflictSelectedRows], [0, 2, 2])
  eq(S.headerText({ uncategorized_total: 10 }, off, false), 'Will categorize 0 of 10 uncategorized  \u00b7  2 conflicts skipped  \u00b7  $0')
  const on = S.tally(CLUSTERS, sel, true)
  eq([on.applyRows, on.applyTotal], [2, 90])
  eq(S.headerText({ uncategorized_total: 10 }, on, true), 'Will categorize 2 of 10 uncategorized  \u00b7  2 conflicts overwritten  \u00b7  $90')
})

test('same-category clusters write nothing and are not counted as applied', () => {
  const sel = Object.assign(S.setAll(CLUSTERS, false), { 'hardware depot#2': true })
  const t = S.tally(CLUSTERS, sel, true)
  eq([t.applyRows, t.selectedClusters, t.sameRows], [0, 1, 1])
  eq(S.selectedTxnIds(CLUSTERS, sel), [])
})

test('selectedTxnIds: every row of selected non-same clusters, deduplicated, in cluster order', () => {
  const sel = S.setAll(CLUSTERS, true)
  const ids = S.selectedTxnIds(CLUSTERS, sel)
  eq(ids.slice(0, 5), [3, 2, 1, 5, 4])
  eq(ids.includes(8), false)            // same-category cluster left out
  eq(ids.length, 3 + 2 + 2 + 60)        // conflict rows stay in (the server counts them as skipped)
  eq(new Set(ids).size, ids.length)
})

test('retainSelection keeps the label-keyed choice across a re-query', () => {
  const old = { 'amazon': true, 'amzn mktp': false, 'hardware depot': true, 'lawn care co': true }
  // The fuzzy cluster is gone at the new threshold; a new cluster appears; amzn mktp is absent.
  const next = [CLUSTERS[0], CLUSTERS[2], { ...CLUSTERS[4], label: 'nightly deli', tier: 'fuzzy', default_selected: true }]
  eq(S.retainSelection(old, next, false), { 'amazon': true, 'hardware depot': true, 'nightly deli': true })
})

test('retainSelection with resetConflicts puts conflicts back on their new default only', () => {
  const old = { 'amazon': false, 'hardware depot': false }
  // The server sends default_selected: true for conflicts when overwrite is on.
  const next = [CLUSTERS[0], Object.assign({}, CLUSTERS[2], { default_selected: true })]
  eq(S.retainSelection(old, next, true), { 'amazon': false, 'hardware depot': true })   // overwrite on -> conflict default true
  eq(S.retainSelection(old, next, false), { 'amazon': false, 'hardware depot': false }) // no reset: the choice is kept
  eq(S.retainSelection({ 'hardware depot': true }, [CLUSTERS[0], CLUSTERS[2]], true), { 'amazon': true, 'hardware depot': false })  // overwrite off
})

test('toggled and setAll return copies', () => {
  const base = { a: true }
  eq(S.toggled(base, 'a'), { a: false })
  eq(base, { a: true })
  eq(S.setAll(CLUSTERS, false)['amazon'], false)
})

test('flatten: collapsed clusters show at most three exemplars; expanded show rows then "more"', () => {
  let items = S.flatten(CLUSTERS, KEYS, {}, {}, {})
  eq(items.filter(i => i.kind === 'exemplar').map(i => i.ckey), ['amazon', 'amazon', 'amazon', 'amzn mktp', 'amzn mktp', 'hardware depot', 'hardware depot', 'hardware depot#2', 'lawn care co', 'lawn care co'])
  eq(items.filter(i => i.kind === 'cluster').length, 5)

  const rows = [{ id: 3, payee: 'A' }, { id: 2, payee: 'B' }]
  items = S.flatten(CLUSTERS, KEYS, { 'amazon': true }, { 'amazon': rows }, { 'amazon': true })
  eq(items.slice(0, 4).map(i => i.kind), ['cluster', 'row', 'row', 'more'])
  eq(items[3].left, 1)
  eq(items[3].loading, true)
  eq(items[1].row, rows[0])
  // Exemplars of the expanded cluster are not shown.
  assert.equal(items.filter(i => i.kind === 'exemplar' && i.ckey === 'amazon').length, 0)
})

test('flatten: a fully loaded expanded cluster has no "more" item', () => {
  const items = S.flatten([CLUSTERS[1]], ['amzn mktp'], { 'amzn mktp': true }, { 'amzn mktp': [{ id: 5 }, { id: 4 }] }, {})
  eq(items.map(i => i.kind), ['cluster', 'row', 'row'])
})

test('paging: 60 rows load as 50 then 10', () => {
  const ids1 = S.nextPageIds(CLUSTERS[4].txn_ids, 0)
  eq(ids1.length, 50)
  const ids2 = S.nextPageIds(CLUSTERS[4].txn_ids, 50)
  eq(ids2.length, 10)
  eq(S.nextPageIds(CLUSTERS[4].txn_ids, 60), [])
  eq(ids2[0], 1051)
})

test('cursor moves over clusters and "more" stops only; pages skip display rows', () => {
  const items = S.flatten(CLUSTERS, KEYS, { 'lawn care co': true }, { 'lawn care co': big.slice(0, 50).map(id => ({ id })) }, {})
  const first = S.firstStop(items)
  eq(items[first].key, 'amazon')
  const second = S.moveStop(items, first, 1, 1)
  eq(items[second].key, 'amzn mktp')
  const lawn = S.indexOfKey(items, 'lawn care co')
  eq(items[lawn].kind, 'cluster')
  const more = S.moveStop(items, lawn, 1, 1)
  eq(items[more].kind, 'more')
  eq(items[more].left, 10)
  eq(S.moveStop(items, more, 1, 5), more)            // end of list: stays
  eq(S.moveStop(items, first, -1, 1), first)         // start of list: stays
  eq(items[S.moveStop(items, more, -1, 2)].key, 'hardware depot#2')   // lawn cluster, then hardware depot#2
  eq(items[S.moveStop(items, more, -1, 10)].key, 'amazon')            // runs out at the top: the first stop
  eq(S.indexOfKey(items, 'gone key'), first)          // a re-query dropped the cursor's cluster
})

test('nothingBeyondSeed: true only when every cluster is a seed', () => {
  eq(S.nothingBeyondSeed(CLUSTERS), false)
  eq(S.nothingBeyondSeed([CLUSTERS[0]]), true)
  eq(S.nothingBeyondSeed([]), true)
})

test('status and summary text', () => {
  eq(S.statusText(CLUSTERS[2]), 'conflict: Groceries')
  eq(S.statusText(CLUSTERS[3]), 'already in this category')
  eq(S.statusText(CLUSTERS[0]), 'uncategorized')
  eq(S.clusterSummary(CLUSTERS[1]), '2 rows  \u00b7  $40.00  \u00b7  2026-10-05..2026-10-20  \u00b7  $15.00..$25.00')
  eq(S.clusterSummary(CLUSTERS[3]), '1 row  \u00b7  $12.00  \u00b7  2026-08-01..2026-08-01  \u00b7  $12.00..$12.00')
  eq(S.rowLine({ date: '2026-11-02', payee: 'AMAZON', amount: 24.99, account: 'Chase' }), '2026-11-02  AMAZON  $24.99  Chase')
})

test('errorMessage prefers the envelope message, then its error code, then the fallback', () => {
  eq(S.errorMessage({ error: 'not_found', message: 'unknown category 99' }, 'x'), 'unknown category 99')
  eq(S.errorMessage({ error: 'bad_request' }, 'x'), 'bad_request')
  eq(S.errorMessage(null, 'HTTP 500'), 'HTTP 500')
})

test('limits match the contract', () => {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', 'state', 'Smart.js'), 'utf8')
  assert.match(src, /var PREVIEW_MAX = 500/)
  assert.match(src, /var APPLY_MAX = 5000/)
  assert.match(src, /var PAGE = 50/)
})

test('indexOfKey: a vanished more-row falls back to its cluster header', () => {
  const items = [
    { kind: 'cluster', key: 'a', ckey: 'a' },
    { kind: 'row', key: 'a/r0', ckey: 'a' },
    { kind: 'cluster', key: 'b', ckey: 'b' },
    { kind: 'row', key: 'b/r0', ckey: 'b' },
  ]
  assert.equal(S.indexOfKey(items, 'more:b'), 2)
  assert.equal(S.indexOfKey(items, 'b/r0'), 3)
  assert.equal(S.indexOfKey(items, 'more:zzz'), S.indexOfKey(items, 'nope'))
})
