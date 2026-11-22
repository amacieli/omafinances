// Node tests for the pure notes helpers in qml/state/Query.js.
// Run: node --test tests/qml/
const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', 'state', 'Query.js'), 'utf8')
const ctx = {}
vm.createContext(ctx)
vm.runInContext(src.replace(/^\.pragma.*$/m, '') + '\nthis.Q = { parseNote, parseNotesFilter, notesTerm, searchParam, SORT_KEYS }', ctx)
const Q = ctx.Q
const plain = x => JSON.parse(JSON.stringify(x))

test('parseNote trims, clears on empty, caps at 255 characters not bytes', () => {
  assert.deepEqual(plain(Q.parseNote('  hi  ')), { note: 'hi' })
  assert.deepEqual(plain(Q.parseNote('   ')), { clear: true })
  assert.deepEqual(plain(Q.parseNote(null)), { clear: true })
  assert.deepEqual(plain(Q.parseNote('é'.repeat(255))), { note: 'é'.repeat(255) })
  assert.ok(Q.parseNote('x'.repeat(256)).error)
  assert.ok(Q.parseNote('a\u0007b').error)
})

test('notes filter and its n: term', () => {
  assert.deepEqual(plain(Q.parseNotesFilter(' trip  home ')), { notes: 'trip home' })
  assert.deepEqual(plain(Q.parseNotesFilter('')), { clear: true })
  assert.ok(Q.parseNotesFilter('a"b').error)
  assert.equal(Q.notesTerm('vacation'), 'n:"vacation"')
  assert.equal(Q.notesTerm('*'), 'n:*')
  assert.equal(Q.notesTerm(''), '')
  assert.equal(Q.searchParam({ slash: 'coffee', notes: '*' }), 'coffee n:*')
  assert.equal(Q.searchParam({ slash: '', notes: '' }), '')
})

test('notes is a sort key after the five numbered ones', () => {
  assert.deepEqual(plain(Q.SORT_KEYS.map(k => k.key)), ['date', 'amount', 'payee', 'account', 'category', 'notes'])
})
