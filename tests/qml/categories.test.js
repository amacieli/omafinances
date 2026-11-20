// Pure logic for the categories phase: path parsing, tree targets (outdent/indent/shift/delete
// cursor), picker entries, mark/visual-range math, fuzzy path matching. No QML needed.
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

const T = load('state/Tree.js')
const M = load('state/Marks.js')
const F = load('common/Fuzzy.js')

// Home
//   Maintenance
//     Plumbing
//   Pets
//     Dogs
// Travel
const ROWS = [
  { id: 1, parent_id: null, name: 'Home', kind: 'expense', depth: 0, count: 0, count_subtree: 5 },
  { id: 2, parent_id: 1, name: 'Maintenance', kind: 'expense', depth: 1, count: 1, count_subtree: 2 },
  { id: 3, parent_id: 2, name: 'Plumbing', kind: 'expense', depth: 2, count: 1, count_subtree: 1 },
  { id: 4, parent_id: 1, name: 'Pets', kind: 'expense', depth: 1, count: 2, count_subtree: 2 },
  { id: 5, parent_id: 4, name: 'Dogs', kind: 'expense', depth: 2, count: 2, count_subtree: 2 },
  { id: 6, parent_id: null, name: 'Travel', kind: 'expense', depth: 0, count: 0, count_subtree: 0 },
]

// ---------------------------------------------------------------- paths and names
test('parsePath splits on slashes, trims, rejects empty segments', () => {
  eq(T.parsePath('Home/Pets/Dogs'), { segments: ['Home', 'Pets', 'Dogs'] })
  eq(T.parsePath('  Home / Pets  '), { segments: ['Home', 'Pets'] })
  eq(T.parsePath('Solo'), { segments: ['Solo'] })
  assert.ok(T.parsePath('').error)
  assert.ok(T.parsePath('   ').error)
  assert.match(T.parsePath('Home//Dogs').error, /empty name/)
  assert.match(T.parsePath('/Home').error, /empty name/)
  assert.match(T.parsePath('Home/').error, /empty name/)
  assert.match(T.parsePath('a'.repeat(81)).error, /at most 80/)
})

test('parseName trims, rejects empty and slash-containing names', () => {
  eq(T.parseName('  Gifts '), { name: 'Gifts' })
  assert.ok(T.parseName('   ').error)
  assert.match(T.parseName('Home/Pets').error, /cannot contain/)
})

test('resolvePath finds the existing prefix and lists the missing names', () => {
  eq(T.resolvePath(ROWS, ['home', 'PETS']), { id: 4, parentId: null, missing: [] })
  eq(T.resolvePath(ROWS, ['Home', 'Pets', 'Cats', 'Tabby']), { id: null, parentId: 4, missing: ['Cats', 'Tabby'] })
  eq(T.resolvePath(ROWS, ['Garden']), { id: null, parentId: null, missing: ['Garden'] })
})

test('pathOf and subtreeIds follow the tree', () => {
  assert.equal(T.pathOf(ROWS, 3), 'Home \u203a Maintenance \u203a Plumbing')
  assert.equal(T.pathOf(ROWS, 99), '')
  eq([...T.subtreeIds(ROWS, 1)].sort((a, b) => a - b), [1, 2, 3, 4, 5])
  eq(T.subtreeIds(ROWS, 6), [6])
})

// ---------------------------------------------------------------- tree targets
test('outdent lands right after the parent under the grandparent', () => {
  // Plumbing (3) is under Maintenance (2), which is under Home (1): Plumbing goes under Home, after Maintenance.
  eq(T.outdentTarget(ROWS, 3), { parent_id: 1, position: 1 })
  // Dogs (5) under Pets (4) under Home: after Pets, which is index 1 of Home's children.
  eq(T.outdentTarget(ROWS, 5), { parent_id: 1, position: 2 })
  assert.equal(T.outdentTarget(ROWS, 1), null)   // already top level
  // Maintenance (2) under Home (1): it goes to the top level, right after Home.
  eq(T.outdentTarget(ROWS, 2), { parent_id: null, position: 1 })
})

test('indent becomes the last child of the previous sibling', () => {
  // Pets (4) follows Maintenance (2): Pets goes under Maintenance, as its last child.
  eq(T.indentTarget(ROWS, 4), { parent_id: 2, position: 1 })
  // Travel (6) follows Home (1), which already has two children.
  eq(T.indentTarget(ROWS, 6), { parent_id: 1, position: 2 })
  assert.equal(T.indentTarget(ROWS, 2), null)    // first among its siblings
})

test('canShift keeps reorder inside the sibling list', () => {
  assert.equal(T.canShift(ROWS, 2, -1), false)
  assert.equal(T.canShift(ROWS, 2, 1), true)
  assert.equal(T.canShift(ROWS, 4, 1), false)
  assert.equal(T.canShift(ROWS, 4, -1), true)
  assert.equal(T.canShift(ROWS, 99, 1), false)
})

test('cursor after delete: first child, else next row, else previous, else null', () => {
  assert.equal(T.neighbourAfterDelete(ROWS, 1), 2)  // children move up; first child takes the cursor
  assert.equal(T.neighbourAfterDelete(ROWS, 3), 4)  // leaf: the row after it
  assert.equal(T.neighbourAfterDelete(ROWS, 6), 5)  // last row: the row before it
  assert.equal(T.neighbourAfterDelete([{ id: 9, parent_id: null, name: 'x' }], 9), null)
})

test('restoreIndex keeps the cursor on its id, else clamps the fallback', () => {
  assert.equal(T.restoreIndex(ROWS, 4, 0), 3)
  assert.equal(T.restoreIndex(ROWS, 99, 2), 2)
  assert.equal(T.restoreIndex(ROWS, 99, 40), 5)
  assert.equal(T.restoreIndex(ROWS, null, 1), 1)
  assert.equal(T.restoreIndex([], 1, 0), -1)
})

test('siblingIndex and childrenOf agree with tree order', () => {
  assert.equal(T.siblingIndex(ROWS, 4), 1)
  assert.equal(T.siblingIndex(ROWS, 1), 0)
  eq(T.childrenOf(ROWS, 1).map(r => r.id), [2, 4])
  eq(T.childrenOf(ROWS, null).map(r => r.id), [1, 6])
})

test('kindNext cycles expense, income, transfer', () => {
  assert.equal(T.kindNext('expense'), 'income')
  assert.equal(T.kindNext('income'), 'transfer')
  assert.equal(T.kindNext('transfer'), 'expense')
  assert.equal(T.kindNext(undefined), 'expense')
})

// ---------------------------------------------------------------- picker entries and fuzzy path matching
test('pickerEntries: specials first, tree indented, exclusions honoured', () => {
  const all = plain(T.pickerEntries(ROWS, { uncategorized: true, uncatCount: 1234, newEntry: true }))
  eq(all.slice(0, 2).map(e => e.key), ['none', 'new'])
  assert.equal(all[0].detail, '1,234 txns')
  const plumb = all.find(e => e.key === '3')
  assert.equal(plumb.match, 'Home \u203a Maintenance \u203a Plumbing')
  assert.equal(plumb.label, '      Plumbing')
  const sub = T.subtreeIds(ROWS, 1)
  const left = plain(T.pickerEntries(ROWS, { exclude: sub }))
  eq(left.map(e => e.key), ['6'])
})

test('fuzzy type-ahead on the full path: "hm" finds Home › Maintenance', () => {
  const entries = plain(T.pickerEntries(ROWS, { uncategorized: true, uncatCount: 0 }))
  const keys = F.filterEntries(entries, 'hm').map(e => e.key)
  assert.ok(keys.includes('2'), 'Home › Maintenance should match hm: ' + keys.join(','))
  assert.ok(!keys.includes('6'), 'Travel should not match hm')
})

// ---------------------------------------------------------------- marks and visual range
test('visual range is inclusive, either direction, and clamped to loaded rows', () => {
  const ids = [10, 11, 12, 13, 14]
  eq(M.rangeOf(1, 3, 5), { lo: 1, hi: 3 })
  eq(M.rangeOf(3, 1, 5), { lo: 1, hi: 3 })
  eq(M.rangeOf(2, 2, 5), { lo: 2, hi: 2 })
  eq(M.rangeOf(0, 99, 5), { lo: 0, hi: 4 })
  assert.equal(M.rangeOf(-1, 2, 5), null)
  assert.equal(M.rangeOf(0, 0, 0), null)
  eq(M.rangeIds(ids, 3, 1), [11, 12, 13])
})

test('effective marks: committed + range, no duplicates, unloaded committed last', () => {
  const ids = [10, 11, 12, 13]
  const committed = { 13: true, 99: true }   // 99 is not on a loaded page
  eq(M.effectiveIds(ids, committed, true, 0, 2), [10, 11, 12, 13, 99])
  eq(M.effectiveIds(ids, committed, false, 0, 2), [13, 99])
  eq(M.effectiveIds(ids, {}, true, -1, 2), [])
})

test('toggle and addAll never mutate the input', () => {
  const base = { 5: true }
  const t = M.toggle(base, 5)
  eq(base, { 5: true })
  eq(t, {})
  eq(M.toggle(t, 7), { 7: true })
  eq(M.addAll(base, [1, 2]), { 5: true, 1: true, 2: true })
  eq(base, { 5: true })
})

test('capIds keeps the first cap ids and reports the cut', () => {
  eq(M.capIds([1, 2, 3], 5), { ids: [1, 2, 3], total: 3, capped: false })
  eq(M.capIds([1, 2, 3], 2), { ids: [1, 2], total: 3, capped: true })
})
