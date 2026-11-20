// Node tests for the pure JS behind the slash search bar (qml/state/Search.js, qml/state/Query.js).
// Run: node --test tests/qml/
const { test } = require('node:test')
const plain = x => JSON.parse(JSON.stringify(x))  // sandbox objects have another Object.prototype
const eq = (actual, expected) => assert.deepStrictEqual(plain(actual), expected)
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

// The QML sources are plain scripts (no module.exports), so evaluate them in a sandbox and pull
// the named top-level functions and variables out.
function load(file, names) {
  const src = fs.readFileSync(path.join(__dirname, '..', '..', 'qml', file), 'utf8')
  const ctx = {}
  vm.createContext(ctx)
  vm.runInContext(src + '\n;this.__x = {' + names.join(',') + '};', ctx)
  return ctx.__x
}

const S = load('state/Search.js', [
  'COMMANDS', 'COMMAND_HELP', 'byteOffset', 'u16Offset', 'replaceRange', 'deleteWordBack',
  'walkHistory', 'parseCommand', 'commandCompletions', 'sortFromValue', 'sortToValue', 'fromParse'
])
const Q = load('state/Query.js', ['emptyFilter', 'chips', 'isActive', 'clearChip', 'buildQuery'])

// ---------------------------------------------------------------- byte <-> UTF-16 offsets
test('byteOffset counts UTF-8 bytes of a prefix', () => {
  assert.equal(S.byteOffset('abc', 2), 2)
  assert.equal(S.byteOffset('Café x', 4), 5)          // é is 2 bytes
  assert.equal(S.byteOffset('Café x', 6), 7)
  assert.equal(S.byteOffset('€', 1), 3)              // U+20AC: 3 bytes
  assert.equal(S.byteOffset('😀!', 2), 4)            // surrogate pair: 4 bytes
  assert.equal(S.byteOffset('😀!', 3), 5)
  assert.equal(S.byteOffset('abc', 99), 3)           // clamped
  assert.equal(S.byteOffset('abc', -4), 0)
})

test('u16Offset maps bytes back to UTF-16 units', () => {
  assert.equal(S.u16Offset('Café x', 5), 4)
  assert.equal(S.u16Offset('😀!', 4), 2)
  assert.equal(S.u16Offset('😀!', 5), 3)
  assert.equal(S.u16Offset('€x', 1, false), 0)        // inside the euro sign: start
  assert.equal(S.u16Offset('€x', 1, true), 1)        // ... or its end when rounding up
  assert.equal(S.u16Offset('abc', 50), 3)
})

test('byte/UTF-16 round trip on non-ASCII text', () => {
  const texts = ['p:"Café Léger" x', 'p:Blåbær😀 -a:Chase', 'c:Ünïcödé/子/日本']
  for (const t of texts) {
    for (let u = 0; u <= t.length; u++) {
      const b = S.byteOffset(t, u)
      // A cut between the two halves of a surrogate pair has no byte position of its own.
      if (u > 0 && u < t.length && t.charCodeAt(u - 1) >= 0xD800 && t.charCodeAt(u - 1) <= 0xDBFF) continue
      assert.equal(S.u16Offset(t, b, false), u, `${JSON.stringify(t)} at ${u}`)
    }
  }
})

// ---------------------------------------------------------------- editing
test('replaceRange replaces the token and adds one trailing space', () => {
  let r = S.replaceRange('p:Bl', 2, 4, 'Blue Bottle')
  assert.equal(r.text, 'p:Blue Bottle ')
  assert.equal(r.cursor, 'p:Blue Bottle '.length)
  r = S.replaceRange('p:Bl tail', 2, 4, 'Blue')
  assert.equal(r.text, 'p:Blue tail')                 // a space already follows: no second one
  assert.equal(r.cursor, 'p:Blue '.length)
})

test('replaceRange works with non-ASCII text before the token', () => {
  const text = 'p:"Café" c:fo'
  const start = text.indexOf('c:')
  const r = S.replaceRange(text, start, text.length, 'c:food/groceries')
  assert.equal(r.text, 'p:"Café" c:food/groceries ')
})

test('deleteWordBack removes the word before the cursor', () => {
  eq(S.deleteWordBack('p:coffee rent', 13), { text: 'p:coffee ', cursor: 9 })
  eq(S.deleteWordBack('abc   ', 6), { text: '', cursor: 0 })
  eq(S.deleteWordBack('one two', 3), { text: ' two', cursor: 0 })
  eq(S.deleteWordBack('', 0), { text: '', cursor: 0 })
})

// ---------------------------------------------------------------- history
test('walkHistory goes older, then newer, and restores the draft', () => {
  const rows = ['p:netflix', 'd:ytd', 'is:pending']       // newest first
  let st = { index: -1, draft: '' }
  let r = S.walkHistory(st.index, st.draft, rows, 'half typed', -1)
  eq(r, { index: 0, draft: 'half typed', text: 'p:netflix' })
  r = S.walkHistory(r.index, r.draft, rows, r.text, -1)
  assert.equal(r.text, 'd:ytd')
  r = S.walkHistory(r.index, r.draft, rows, r.text, -1)
  assert.equal(r.text, 'is:pending')
  assert.equal(S.walkHistory(r.index, r.draft, rows, r.text, -1), null)   // oldest reached
  r = S.walkHistory(r.index, r.draft, rows, r.text, 1)
  assert.equal(r.text, 'd:ytd')
  r = S.walkHistory(r.index, r.draft, rows, r.text, 1)
  r = S.walkHistory(r.index, r.draft, rows, r.text, 1)
  eq(r, { index: -1, draft: 'half typed', text: 'half typed' })
  assert.equal(S.walkHistory(-1, 'x', rows, 'x', 1), null)              // already at the draft
})

test('walkHistory with empty history does nothing', () => {
  assert.equal(S.walkHistory(-1, '', [], 'abc', -1), null)
})

// ---------------------------------------------------------------- commands
test('parseCommand recognises commands and reports errors', () => {
  assert.equal(S.parseCommand('p:coffee'), null)
  eq(S.parseCommand(':save Coffee'), { cmd: 'save', arg: 'Coffee' })
  eq(S.parseCommand(':view  "Big spends"'), { cmd: 'view', arg: 'Big spends' })
  eq(S.parseCommand(':VIEWS'), { cmd: 'views', arg: '' })
  eq(S.parseCommand(':clear'), { cmd: 'clear', arg: '' })
  assert.match(S.parseCommand(':save').error, /usage: :save <name>/)
  assert.match(S.parseCommand(':delete   ').error, /usage: :delete/)
  assert.match(S.parseCommand(':frob').error, /unknown command :frob/)
  assert.match(S.parseCommand(':').error, /type a command/)
})

test('commandCompletions completes command names and view names', () => {
  const views = ['Coffee run', 'Big spends', 'Rent']
  let c = S.commandCompletions(':', 1, views)
  assert.equal(c.start, 1)
  eq(c.items.map(i => i.insert), plain(S.COMMANDS))
  c = S.commandCompletions(':VI', 3, views)
  eq(c.items.map(i => i.insert), ['view', 'views'])
  assert.equal(c.start, 1)
  assert.equal(c.end, 3)
  c = S.commandCompletions(':view ', 6, views)
  eq(c.items.map(i => i.label), views)
  assert.equal(c.start, 6)
  c = S.commandCompletions(':delete big', 11, views)
  eq(c.items.map(i => i.label), ['Big spends'])
  assert.equal(c.start, 8)
  assert.equal(c.end, 11)
  c = S.commandCompletions(':view coffee run', 6, views)
  assert.equal(c.items[0].insert, '"Coffee run"')   // names with spaces are quoted
  assert.equal(S.commandCompletions('p:x', 3, views), null)
})

// ---------------------------------------------------------------- sort values
test('sortFromValue and sortToValue', () => {
  eq(S.sortFromValue('-amount'), { key: 'amount', dir: 'desc' })
  eq(S.sortFromValue('payee'), { key: 'payee', dir: 'asc' })
  eq(S.sortFromValue('date:desc'), { key: 'date', dir: 'desc' })
  eq(S.sortFromValue({ key: 'category', dir: 'asc' }), { key: 'category', dir: 'asc' })
  assert.equal(S.sortFromValue('bogus'), null)
  assert.equal(S.sortFromValue(null), null)
  assert.equal(S.sortToValue({ key: 'amount', dir: 'desc' }), '-amount')
  assert.equal(S.sortToValue({ key: 'payee', dir: 'asc' }), 'payee')
})

// ---------------------------------------------------------------- /search/parse reply
test('fromParse converts byte offsets for the error span and completions', () => {
  const text = 'p:Café zz:1'
  const resp = {
    ok: false, canonical: '', matched: null, terms: [], sort: null,
    parse_error: { pos: 8, len: 4, message: 'unknown field zz' },
    completions: { start: 8, end: 10, items: [{ insert: 'zz:', label: 'zz:', detail: 'x' }] }
  }
  const p = S.fromParse(text, resp)
  assert.equal(p.ok, false)
  eq(p.error, { start: 7, end: 11, message: 'unknown field zz' })
  assert.equal(text.substring(p.error.start, p.error.end), 'zz:1')
  assert.equal(text.substring(p.completions.start, p.completions.end), 'zz')
  assert.equal(p.completions.items[0].insert, 'zz:')
})

test('fromParse reads matched count, canonical form and sort term', () => {
  const p = S.fromParse('sort:-amount "Café"', {
    ok: true, canonical: 'sort:-amount "Café"', matched: 42,
    terms: [{ text: 'Café', neg: false, field: '', value: 'Café' }, { text: 'sort:-amount', field: 'sort', value: '-amount' }],
    sort: { key: 'amount', dir: 'desc' }, parse_error: null
  })
  assert.equal(p.ok, true)
  assert.equal(p.matched, 42)
  assert.equal(p.sortValue, '-amount')
  assert.equal(p.error, null)
  eq(p.completions, { start: 0, end: 0, items: [] })
  const none = S.fromParse('p:x', { ok: true, canonical: 'p:x', matched: null, terms: [] })
  assert.equal(none.matched, null)
  assert.equal(none.sortValue, null)
})

test('fromParse treats a reply with parse_error as not ok', () => {
  const p = S.fromParse('d:', { ok: true, parse_error: { pos: 0, len: 2, message: 'empty date' } })
  assert.equal(p.ok, false)
  assert.equal(p.error.message, 'empty date')
})

// ---------------------------------------------------------------- Query.js: slash query in the filter
test('slash query is part of the filter: query string, chip, clearing', () => {
  const f = Object.assign(Q.emptyFilter(), { slash: 'p:netflix', from: '2026-01-01', to: '2026-02-01' })
  assert.equal(Q.isActive(f), true)
  const chips = Q.chips(f)
  assert.equal(chips[0].id, 'search')
  assert.equal(chips[0].label, 'p:netflix')
  const url = Q.buildQuery(f, { key: 'date', dir: 'desc' }, 0, 200)
  assert.match(url, /[?&]s=p%3Anetflix(&|$)/)
  assert.match(url, /from=2026-01-01/)
  assert.equal(Q.clearChip(f, 'search').slash, '')
  assert.equal(Q.buildQuery(Q.emptyFilter(), { key: 'date', dir: 'desc' }, 0, 200).includes('s='), false)
})
