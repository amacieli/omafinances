// Multi-select math for the transaction table. Pure functions over row indices and transaction
// ids. Committed marks are an object {txid: true}. A visual range is an anchor row index plus the
// cursor's row index, over the loaded rows (ids: transaction ids in table order).

// The inclusive {lo, hi} row range between anchor and cursor, clamped to count; null if empty.
function rangeOf(anchor, cursor, count) {
  if (count <= 0 || anchor < 0 || cursor < 0) return null
  var a = Math.min(anchor, count - 1)
  var b = Math.min(cursor, count - 1)
  return { lo: Math.min(a, b), hi: Math.max(a, b) }
}

// Transaction ids inside the visual range, in table order.
function rangeIds(ids, anchor, cursor) {
  var r = rangeOf(anchor, cursor, ids.length)
  return r ? ids.slice(r.lo, r.hi + 1) : []
}

// The marks in effect: committed marks plus, when visualOn, the visual range. Loaded rows come
// first in table order, then committed marks for rows that are not loaded. No duplicates.
function effectiveIds(ids, committed, visualOn, anchor, cursor) {
  var seen = {}
  var out = []
  function add(id) {
    var k = String(id)
    if (seen[k]) return
    seen[k] = true
    out.push(Number(id))
  }
  var r = visualOn ? rangeOf(anchor, cursor, ids.length) : null
  for (var i = 0; i < ids.length; i++) {
    var inRange = r !== null && i >= r.lo && i <= r.hi
    if (inRange || committed[String(ids[i])]) add(ids[i])
  }
  Object.keys(committed).forEach(function(k) { add(k) })
  return out
}

// A copy of `committed` with id toggled.
function toggle(committed, id) {
  var copy = Object.assign({}, committed)
  if (copy[String(id)]) delete copy[String(id)]
  else copy[String(id)] = true
  return copy
}

// A copy of `committed` with every id in ids added.
function addAll(committed, ids) {
  var copy = Object.assign({}, committed)
  ids.forEach(function(id) { copy[String(id)] = true })
  return copy
}

// The first `cap` ids, and whether the list was cut.
function capIds(ids, cap) {
  return { ids: ids.slice(0, cap), total: ids.length, capped: ids.length > cap }
}
