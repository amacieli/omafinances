// Categorize with preview (Phase 4): pure helpers for the preview overlay (views/SmartPreview.qml)
// and AppShell. No QML here. tests/qml/smart.test.js covers them.
//
// Cluster identity is the cluster's label (its normalized payee). A label can repeat across
// tiers, so a repeat gets "#2", "#3" ... in response order. Keys are what selection, the cursor
// and row expansion are kept by when a re-query (threshold, overwrite) replaces the clusters.

var THRESHOLD_MIN = 0.5
var THRESHOLD_MAX = 1.0
var THRESHOLD_STEP = 0.05
var DEFAULT_THRESHOLD = 0.75
var PAGE = 50            // rows per /smart/rows request and per "more" step
var EXEMPLAR_LINES = 3   // exemplars shown under a collapsed cluster
var PREVIEW_MAX = 500    // /smart/preview ids limit
var APPLY_MAX = 5000     // /smart/apply txn_ids limit

// Clamps to 0.5..1.0 and rounds to hundredths (float steps drift otherwise).
function clampThreshold(t) {
  var v = Number(t)
  if (!isFinite(v)) v = DEFAULT_THRESHOLD
  v = Math.round(v * 100) / 100
  return Math.max(THRESHOLD_MIN, Math.min(THRESHOLD_MAX, v))
}

// [ (dir -1) and ] (dir +1): one step of 0.05, clamped.
function stepThreshold(t, dir) {
  return clampThreshold(Number(t) + dir * THRESHOLD_STEP)
}

function thresholdPercent(t) {
  return Math.round(clampThreshold(t) * 100) + "%"
}

// Keys for a cluster list, in order: the label, with "#n" added to repeats.
function clusterKeys(clusters) {
  var seen = {}
  var out = []
  for (var i = 0; i < clusters.length; i++) {
    var base = String(clusters[i].label)
    var k = base
    var n = 1
    while (seen[k]) { n++; k = base + "#" + n }
    seen[k] = true
    out.push(k)
  }
  return out
}

function fmtN(n) {
  return String(Math.round(Number(n) || 0)).replace(/\B(?=(\d{3})+(?!\d))/g, ",")
}

// "$18,402" (whole dollars, as the header shows them). Negative totals keep their sign.
function money(n) {
  var v = Math.round(Number(n) || 0)
  return (v < 0 ? "-$" : "$") + fmtN(Math.abs(v))
}

// "$1,234.56" for row amounts.
function moneyCents(n) {
  var v = Number(n) || 0
  var parts = Math.abs(v).toFixed(2).split(".")
  return (v < 0 ? "-$" : "$") + parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ",") + "." + parts[1]
}

// Selection after a re-query. A cluster keeps the selection it had under the same key; a new
// cluster takes its default_selected. resetConflicts (the overwrite toggle) puts conflict
// clusters back on their new default, since that default is what the toggle changes.
function retainSelection(oldSelected, newClusters, resetConflicts) {
  var keys = clusterKeys(newClusters)
  var old = oldSelected || {}
  var sel = {}
  for (var i = 0; i < newClusters.length; i++) {
    var c = newClusters[i]
    var k = keys[i]
    if (resetConflicts && c.status === "conflict") sel[k] = !!c.default_selected
    else if (Object.prototype.hasOwnProperty.call(old, k)) sel[k] = !!old[k]
    else sel[k] = !!c.default_selected
  }
  return sel
}

// A copy of `selected` with every cluster set to value.
function setAll(clusters, value) {
  var keys = clusterKeys(clusters)
  var sel = {}
  for (var i = 0; i < keys.length; i++) sel[keys[i]] = !!value
  return sel
}

// A copy of `selected` with key flipped.
function toggled(selected, key) {
  var copy = Object.assign({}, selected)
  copy[key] = !copy[key]
  return copy
}

// What applying the selection does, counted the way /smart/apply counts it.
//   applyRows / applyTotal: rows that get written (uncategorized clusters, and conflict clusters
//                           when overwrite is on). Same-category clusters write nothing.
//   conflictRows:           rows in conflict clusters (all of them; they are skipped without overwrite)
//   conflictSelectedRows:   conflict rows whose cluster is selected (overwritten, with overwrite on)
function tally(clusters, selected, overwrite) {
  var keys = clusterKeys(clusters)
  var t = { applyRows: 0, applyTotal: 0, selectedClusters: 0, conflictRows: 0, conflictSelectedRows: 0, sameRows: 0 }
  for (var i = 0; i < clusters.length; i++) {
    var c = clusters[i]
    var on = !!selected[keys[i]]
    if (c.status === "conflict") {
      t.conflictRows += Number(c.count) || 0
      if (on) t.conflictSelectedRows += Number(c.count) || 0
    }
    if (c.status === "same") t.sameRows += Number(c.count) || 0
    if (!on) continue
    t.selectedClusters++
    if (c.status === "same") continue
    if (c.status === "conflict" && !overwrite) continue
    t.applyRows += Number(c.count) || 0
    t.applyTotal += Number(c.total) || 0
  }
  return t
}

// "Will categorize 214 of 1,194 uncategorized  ·  11 conflicts skipped  ·  $18,402"
function headerText(summary, t, overwrite) {
  var s = summary || {}
  var parts = ["Will categorize " + fmtN(t.applyRows) + " of " + fmtN(s.uncategorized_total) + " uncategorized"]
  if (overwrite) parts.push(fmtN(t.conflictSelectedRows) + " conflicts overwritten")
  else parts.push(fmtN(t.conflictRows) + " conflicts skipped")
  parts.push(money(t.applyTotal))
  return parts.join("  \u00b7  ")
}

// The transaction ids /smart/apply gets: every row of each selected cluster, except same-category
// clusters (they write nothing). Conflict rows stay in, so the server counts them as skipped.
// Deduplicated, in cluster order.
function selectedTxnIds(clusters, selected) {
  var keys = clusterKeys(clusters)
  var seen = {}
  var out = []
  for (var i = 0; i < clusters.length; i++) {
    var c = clusters[i]
    if (!selected[keys[i]] || c.status === "same") continue
    for (var j = 0; j < c.txn_ids.length; j++) {
      var id = Number(c.txn_ids[j])
      if (seen[id]) continue
      seen[id] = true
      out.push(id)
    }
  }
  return out
}

// True when the preview found nothing beyond the seed rows (the selected rows themselves).
function nothingBeyondSeed(clusters) {
  for (var i = 0; i < clusters.length; i++) if (clusters[i].tier !== "seed") return false
  return true
}

// "uncategorized", "already Home › Maintenance", "conflict: Groceries, Dining".
function statusText(c) {
  if (c.status === "conflict") return "conflict: " + (c.conflict_categories || []).join(", ")
  if (c.status === "same") return "already in this category"
  return "uncategorized"
}

// "12 rows  ·  $340.10  ·  2026-09-02..2026-11-18  ·  $9.00..$88.40"
function clusterSummary(c) {
  var n = Number(c.count) || 0
  return fmtN(n) + (n === 1 ? " row" : " rows") + "  \u00b7  " + moneyCents(c.total)
       + "  \u00b7  " + c.date_min + ".." + c.date_max
       + "  \u00b7  " + moneyCents(c.amount_min) + ".." + moneyCents(c.amount_max)
}

// One transaction as a line: "2026-11-02  AMAZON MKTP  $24.99  Chase Sapphire"
function rowLine(r) {
  return String(r.date || "") + "  " + String(r.payee || "") + "  " + moneyCents(r.amount)
       + "  " + String(r.account || "")
}

// Ids to ask /smart/rows for: the next page of a cluster, given how many rows are loaded.
function nextPageIds(txnIds, loaded) {
  return txnIds.slice(loaded, loaded + PAGE)
}

// The list the overlay shows, flattened. Each item: {kind, key, ckey, ...}.
//   cluster    the cluster header (cursor stop). ci: index in clusters
//   exemplar   up to EXEMPLAR_LINES lines under a collapsed cluster (display only)
//   row        a loaded row under an expanded cluster (display only). row: the row
//   more       "N more": cursor stop; Enter loads the next PAGE rows. left, loaded, loading
// ckey is the owning cluster's key (Space toggles it from any item of the cluster).
function flatten(clusters, keys, expanded, rowsByKey, loading) {
  var items = []
  for (var i = 0; i < clusters.length; i++) {
    var c = clusters[i]
    var k = keys[i]
    items.push({ kind: "cluster", key: k, ckey: k, ci: i })
    if (expanded && expanded[k]) {
      var rows = (rowsByKey && rowsByKey[k]) || []
      for (var j = 0; j < rows.length; j++)
        items.push({ kind: "row", key: k + "/r" + j, ckey: k, row: rows[j] })
      var left = c.txn_ids.length - rows.length
      if (left > 0)
        items.push({ kind: "more", key: "more:" + k, ckey: k, left: left, loaded: rows.length,
                     loading: !!(loading && loading[k]) })
    } else {
      var ex = c.exemplars || []
      var n = Math.min(EXEMPLAR_LINES, ex.length)
      for (var e = 0; e < n; e++)
        items.push({ kind: "exemplar", key: k + "/e" + e, ckey: k, row: ex[e] })
    }
  }
  return items
}

function isStop(item) {
  return !!item && (item.kind === "cluster" || item.kind === "more")
}

// The cursor's next resting place: `steps` stops from idx in direction dir (+1/-1), stopping at
// the last stop before the edge. Returns idx when there is no stop that way.
function moveStop(items, idx, dir, steps) {
  var last = idx
  var left = steps
  for (var j = idx + dir; j >= 0 && j < items.length; j += dir) {
    if (!isStop(items[j])) continue
    last = j
    left--
    if (left <= 0) break
  }
  return last
}

// Index of the first cursor stop, or -1 when there is none.
function firstStop(items) {
  for (var i = 0; i < items.length; i++) if (isStop(items[i])) return i
  return -1
}

// Index of the item with this key, or the first stop when it is gone (a re-query dropped it).
function indexOfKey(items, key) {
  for (var i = 0; i < items.length; i++) if (items[i].key === key) return i
  // A "more" row that vanished (its cluster is fully loaded): fall back to that cluster's header.
  if (String(key).indexOf("more:") === 0) {
    var ck = String(key).slice(5)
    for (var j = 0; j < items.length; j++) if (items[j].kind === "cluster" && items[j].ckey === ck) return j
  }
  return firstStop(items)
}

// Cluster for a key, or null.
function clusterByKey(clusters, keys, key) {
  for (var i = 0; i < keys.length; i++) if (keys[i] === key) return clusters[i]
  return null
}

// The error text from a backend error envelope {error, message}, else the fallback.
function errorMessage(j, fallback) {
  if (j && j.message) return String(j.message)
  if (j && j.error) return String(j.error)
  return fallback
}
