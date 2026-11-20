// Type-ahead matching for the pickers. Every query character must appear in order in the text
// (case-insensitive). Ranking: a prefix match first, then a plain substring, then scattered.
// Returns a score (higher is better), or -1 for no match. An empty query matches everything.

function fuzzyScore(query, text) {
  var q = String(query || "").toLowerCase().replace(/\s+/g, "")
  var t = String(text || "").toLowerCase()
  if (q === "") return 0
  if (t.indexOf(q) === 0) return 300 - t.length / 1000
  var at = t.indexOf(q)
  if (at >= 0) return 200 - at / 1000
  var j = 0, first = -1, last = -1
  for (var i = 0; i < t.length && j < q.length; i++) {
    if (t.charAt(i) === q.charAt(j)) {
      if (first < 0) first = i
      last = i
      j++
    }
  }
  if (j < q.length) return -1
  return 100 - (last - first)
}

// Filters and ranks a list of entries {key,label,detail,match} by `query`. The primary text is
// `match` when the entry has one (a category's full path, so "hm" finds "Home › Maintenance"),
// else `label`; the detail counts a little less. Entries that did not match are dropped; order is
// by score, then the original order.
function filterEntries(entries, query) {
  var scored = []
  for (var i = 0; i < entries.length; i++) {
    var e = entries[i]
    var primary = e.match !== undefined ? e.match : e.label
    var s = Math.max(fuzzyScore(query, primary), fuzzyScore(query, e.detail || "") - 50)
    if (s >= 0) scored.push({ e: e, s: s, i: i })
  }
  if (String(query || "").trim() !== "") scored.sort(function(a, b) { return b.s - a.s || a.i - b.i })
  return scored.map(function(x) { return x.e })
}
