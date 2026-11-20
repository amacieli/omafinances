// Node tests for qml/keys/Keymap.js: no key is bound twice in one scope, and the Phase 5 keys
// (D, R, the dashboard and review overlays, Ctrl-y) resolve where the contract puts them.
// Run: node --test tests/qml/
const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

// The file's top-level vars and functions, exported for the tests (Keymap.js has no imports).
function load(file) {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', file), 'utf8')
  const names = [...src.matchAll(/^function (\w+)\(/gm)].map(m => m[1])
  const consts = [...src.matchAll(/^var (\w+) =/gm)].map(m => m[1])
  const ctx = {}
  vm.runInNewContext(src + '\n;__out = {' + names.concat(consts).join(',') + '};', ctx)
  return ctx.__out
}

const K = load('keys/Keymap.js')

test('no key is bound twice within one scope', () => {
  const seen = new Map()
  for (const b of K.BINDINGS) {
    for (const k of b.keys) {
      const key = b.scope + ' ' + k
      assert.ok(!seen.has(key), 'duplicate: ' + key + ' (' + seen.get(key) + ' and ' + b.id + ')')
      seen.set(key, b.id)
    }
  }
})

test('every binding names a scope that has a title and an order slot', () => {
  for (const b of K.BINDINGS) {
    assert.ok(K.SCOPE_TITLES[b.scope], 'no title for scope ' + b.scope + ' (' + b.id + ')')
    assert.ok(K.SCOPE_ORDER.includes(b.scope), 'scope ' + b.scope + ' missing from SCOPE_ORDER')
  }
})

test('D and R open the overlays from the table, and the overlays own their keys', () => {
  assert.equal(K.resolve(['table', 'global'], 'D'), 'dash.open')
  assert.equal(K.resolve(['table', 'global'], 'R'), 'review.open')
  assert.equal(K.resolve(['dash'], 'Tab'), 'dash.next')
  assert.equal(K.resolve(['dash'], 'S-Tab'), 'dash.prev')
  assert.equal(K.resolve(['dash'], 'Esc'), 'dash.close')
  assert.equal(K.resolve(['dash'], 't'), 'dash.transfers')
  assert.equal(K.resolve(['review'], '1'), 'review.pick1')
  assert.equal(K.resolve(['review'], '3'), 'review.pick3')
  assert.equal(K.resolve(['review'], 'c'), 'review.categorize')
  assert.equal(K.resolve(['review'], 'C'), 'review.smart')
  assert.equal(K.resolve(['review'], 's'), 'review.skip')
  assert.equal(K.resolve(['review'], 'Backspace'), 'review.prev')
  assert.equal(K.resolve(['review'], 'u'), 'review.undo')
  assert.equal(K.resolve(['review'], 'Esc'), 'review.close')
})

test('Ctrl-y accepts the suggestion from the detail pane only', () => {
  assert.equal(K.resolve(['detail', 'global'], 'C-y'), 'suggest.accept')
  assert.equal(K.resolve(['table', 'global'], 'C-y'), '')
})

test('the overlay keys do not leak into the table scope', () => {
  assert.equal(K.resolve(['table', 'global'], '1'), 'sort.date')
  assert.equal(K.resolve(['table', 'global'], 's'), 'sort.menu')
})
