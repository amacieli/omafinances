# Phase 4 contract: categorize with preview

Flow: select rows (or the cursor row) -> `C` -> pick a category (existing picker) -> **preview screen** (clusters,
exemplars, conflicts) -> toggle clusters -> `y` applies in ONE journal batch (`u` undoes all of it) -> optional
"also make this a rule". Nothing is written until `/smart/apply`. Never writes to Lunch Money.

All routes: token-guarded like the rest; POST with JSON bodies; errors `{error,message}` 400/404/409/503 as in `api/edit.rs`.
Amounts: positive = money out. Dates `YYYY-MM-DD`.

## POST /smart/preview   (read-only, no journal)
Request: `{ "ids":[int,...1..500], "category_id":int, "threshold":0.75, "overwrite":false }`
 - `threshold` 0.5..1.0 (default 0.75): minimum confidence for a candidate to be listed. 400 outside the range.
 - 404 unknown category_id; ids that don't exist are ignored (400 if none exist).
Response:
```
{ "category": {"id":int,"path":"A › B"},
  "threshold": 0.75,
  "clusters": [ {
      "id": "c1",                        // stable within this response
      "label": "amazon",                 // normalized payee of the cluster
      "tier": "seed"|"exact"|"fuzzy"|"recurring"|"amount_band",
      "confidence": 0.0..1.0,            // seed cluster = 1.0
      "reason": "same normalized payee" | "payee 0.86 similar to 'amazon'" | "same recurring id" | "...",
      "count": int, "total": float, "date_min": "...", "date_max": "...",
      "amount_min": float, "amount_max": float,
      "status": "uncategorized"|"conflict"|"same",   // conflict = rows already in a DIFFERENT category;
                                                      // same = already in the target category
      "conflict_categories": ["A › B", ...],          // distinct paths, [] unless conflict
      "default_selected": bool,   // seed + uncategorized: true; conflict: true only when overwrite=true; same: false
      "exemplars": [ {"id","date","payee","amount","account","category_path":null|"..","why":"earliest"|"latest"|"median"|"outlier"} ],  // 1..4, distinct rows
      "txn_ids": [int,...]        // every row in the cluster, sorted by date desc
  } ],                            // sorted: seed first, then confidence desc, count desc
  "summary": { "matched": int, "selected": int, "selected_total": float,
               "uncategorized_total": int, "conflicts": int, "same": int } }
```
Rules: a txn appears in at most one cluster (highest tier/confidence wins); the seed rows are always in a `seed`
cluster (split by normalized payee when several payees are marked: one seed cluster per payee). A cluster is
homogeneous in `status` (split by status if members differ). Deterministic output for identical input.

## POST /smart/rows   (read-only)
Request `{ "ids":[...1..500] }` -> `{ "rows":[{"id","date","payee","amount","account","category_path"}] }` in the request order; unknown ids omitted.
Used to expand a cluster.

## POST /smart/apply
Request: `{ "category_id":int, "txn_ids":[...1..5000], "overwrite":false, "make_rule": null | {"name":"optional"} }`
 - Without `overwrite`, rows that already have a DIFFERENT category are skipped (counted in `skipped_conflicts`).
 - Whole thing (category writes + optional rule insert) is ONE DB transaction and ONE `change_batch` (kind `smart`),
   so `POST /undo` reverts rows and rule together, `POST /redo` re-applies.
 - `make_rule`: inserts a `rule` row `{name, enabled:1, match_json, category_id, position:last}`; `match_json` =
   `{"payee_norm": "<seed normalized payee>"}` (when seeds had several payees: `{"payee_norm_any":[...]}`), plus
   `"amount_min"/"amount_max"` only when the seed rows share a tight band (all within 10% of median). Duplicate rule
   (same match_json+category) is not inserted again (no error, `rule_id` returned as the existing one).
Response 200: `{ "ok":true, "batch_id":int|null, "label":"Categorize 214 transactions as Home › Maintenance (+rule)", "changed":int, "skipped":int, "skipped_conflicts":int, "rule_id":int|null }`.
Cache is refreshed before responding (like edit routes).

## GET /rules   (read-only listing for visibility)
`{ "rules":[{"id","position","name","enabled":bool,"match":{...},"category_id","category_path","hits"}] }`

## Normalizer (`src/smart/normalize.rs`)
lowercase; strip prefixes `pos `, `sq *`, `tst*`, `paypal *`, `pp*`, `amzn mktp us*`->`amazon`, `debit card purchase`; strip
store/terminal numbers and `#123`, card tails `xxxx1234`, dates, trailing `ST`/city+state tokens when they follow a number
or are a US state code at the end; collapse whitespace/punctuation. The `payee_alias` table (exact or regex, `is_regex`)
is applied FIRST and wins. Pure function `normalize(raw, &aliases) -> String`.

## Tiers (candidate search)
(a) `exact`: equal normalized payee. (b) `fuzzy`: trigram Jaccard combined with Jaro-Winkler >= threshold (confidence = that score).
(c) `recurring`: same `recurring_id` as a seed row (confidence 0.9). (d) `amount_band`: shares >=1 non-stop payee token with the
seed AND amount within 25% of the seed median (confidence 0.6..0.8, listed only if >= threshold). Never match across the sign
(income vs expense) of the seed.

## Frontend contract (QML)
 - Key `C` (scope table) = categorize with preview; `c` stays the plain picker. After the picker returns a category,
   call preview; show the preview overlay; `Space` toggles the cluster, `a` all, `n` none, `Enter` expands a cluster
   (lazy via /smart/rows, paged by 50), `[`/`]` threshold -/+ 0.05 (re-queries preview, keeps selection by cluster label),
   `o` toggles overwrite (re-queries), `r` toggles "also make this a rule", `y` apply, `Esc` cancel (no write).
 - Header: "Will categorize 214 of 1,194 uncategorized  .  11 conflicts skipped  .  $18,402". Conflict clusters use the
   theme's warning/orange color; same-category clusters are dimmed.
 - After apply: toast with label + "u to undo"; table reloads and keeps cursor.
 - New overlays must go through AppShell's `restoreFocus()` and be added to the `overlayOpen` list.
