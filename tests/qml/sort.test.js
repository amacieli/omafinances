// Sort state for the transaction table: each column toggles between ascending and descending only,
// and keys 1-5 remember the direction each column was last in.
const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const plain = x => JSON.parse(JSON.stringify(x))
const eq = (actual, expected) => assert.deepStrictEqual(plain(actual), expected)

function load(file, names) {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', file), 'utf8')
  const ctx = {}
  vm.runInNewContext(src + '\n;__out = {' + names.join(',') + '};', ctx)
  return ctx.__out
}

const Q = load('state/Query.js', ['defaultSortMemory', 'nextSort', 'rememberSort', 'sortInfo'])

// Mirrors AppShell.applySort: the state is the current sort plus the per-column memory.
function press(state, key) {
  const sort = Q.nextSort(state.sort, key, state.mem)
  return { sort, mem: Q.rememberSort(state.mem, sort) }
}

test('default sort is date descending, with each column default', () => {
  const mem = Q.defaultSortMemory()
  eq(mem, { date: 'desc', amount: 'desc', payee: 'asc', account: 'asc', category: 'asc', notes: 'asc' })
  eq(Q.nextSort({ key: 'date', dir: 'desc' }, 'date', mem), { key: 'date', dir: 'asc' })
})

test('pressing the current column key only reverses it', () => {
  let st = { sort: { key: 'date', dir: 'desc' }, mem: Q.defaultSortMemory() }
  st = press(st, 'date'); eq(st.sort, { key: 'date', dir: 'asc' })
  st = press(st, 'date'); eq(st.sort, { key: 'date', dir: 'desc' })
  st = press(st, 'date'); eq(st.sort, { key: 'date', dir: 'asc' })
})

test('switching away and back keeps each column on its own asc/desc toggle', () => {
  let st = { sort: { key: 'date', dir: 'desc' }, mem: Q.defaultSortMemory() }
  st = press(st, 'date')            // date asc
  st = press(st, 'amount')          // amount desc (its default)
  eq(st.sort, { key: 'amount', dir: 'desc' })
  st = press(st, 'date')            // back to date: must be asc, the direction it was left in
  eq(st.sort, { key: 'date', dir: 'asc' })
  st = press(st, 'amount')          // amount: remembered desc, not reset
  eq(st.sort, { key: 'amount', dir: 'desc' })
  st = press(st, 'amount')
  eq(st.sort, { key: 'amount', dir: 'asc' })
  st = press(st, 'payee')           // payee default asc
  eq(st.sort, { key: 'payee', dir: 'asc' })
  st = press(st, 'amount')          // amount remembered asc
  eq(st.sort, { key: 'amount', dir: 'asc' })
})

test('a direction set by a slash search is remembered by its column', () => {
  let mem = Q.rememberSort(Q.defaultSortMemory(), { key: 'payee', dir: 'desc' })
  eq(Q.nextSort({ key: 'date', dir: 'desc' }, 'payee', mem), { key: 'payee', dir: 'desc' })
})
