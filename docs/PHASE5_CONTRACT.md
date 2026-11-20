# Phase 5 contract: intelligence and insights

Everything is deterministic, in-process, read-only except through the EXISTING undoable `POST /transactions/categorize`
(accepting a suggestion = that call). No new tables, no triggers, no FTS. Reuses `smart::normalize` + `smart::similarity`.
Same guard/error envelope/amount convention as before (positive = money out, dates YYYY-MM-DD). Routes are GET unless noted;
list params are query strings, bad values are 400 `invalid`, 503 without DB. Output is deterministic (stable sorts, ties by id).

## Suggest engine (`src/insight/suggest.rs`)
Training set = transactions with a local category (`category_id` set). Features: normalized-payee tokens (+ whole normalized payee),
amount bucket (log10 bands, sign included), account id, day-of-month bucket (1-7,8-14,15-21,22+), and LM category name when present.
Model: multinomial naive Bayes with Laplace smoothing, log-space, over the local category ids (leaf = the assigned category).
Rules first: an enabled `rule` whose match_json matches (payee_norm / payee_norm_any, amount_min/max) yields an option with
`source:"rule"`, confidence 1.0 (first rule by position wins) before model options.
**Accuracy gate (must be measured, never assumed):** deterministic 5-fold cross-validation over the labeled rows (fold = id % 5),
reporting top1/top3 hit rates. `enabled` is true only when `labeled >= 30` AND `top3 >= 0.70`. Otherwise suggestions are still
returned as `[]` with `enabled:false` and a `reason` string. Cache the trained model + CV result inside AppState, invalidated on
every cache refresh (build lazily on first request; must stay < 50 ms for 50k rows trained, test it with generated rows).

`POST /suggest` body `{ "ids":[1..200] }` ->
`{ "enabled":bool, "reason":null|"only 3 labeled rows (need 30)"|"top-3 accuracy 0.52 below 0.70",
   "accuracy": {"labeled":int,"folds":5,"top1":0.0..1.0,"top3":0.0..1.0},
   "suggestions":[{"id":int,"options":[{"category_id":int,"path":"A › B","confidence":0..1,"source":"rule"|"model"}]}] }`
 (up to 3 options per row, only rows that are currently uncategorized, options below confidence 0.30 dropped, rule options always kept.)
`GET /suggest/accuracy` -> the `accuracy`/`enabled`/`reason` part only.

## Review queue (`src/insight/review.rs`)
`GET /review?limit=50&offset=0` -> `{ "mode":"suggest"|"frequency", "remaining":int, "rows":[{"id","date","payee","amount","account","payee_count":int,"options":[...as above...]}] }`
 - Only uncategorized rows. `mode:"suggest"` when the engine is enabled: sorted by top-option confidence desc, then date desc.
   `mode:"frequency"` otherwise (bootstrap): sorted by normalized-payee frequency among uncategorized desc (biggest payee groups first,
   groups kept together, then date desc), `options` = [] (rule options still included if a rule matches).
 - limit 1..200, offset >= 0.

## Insights
`GET /insights/recurring` -> `{ "series":[{"payee":"netflix","period_days":30,"period":"monthly","count":14,"amount_median":15.49,"amount_cv":0.0,"last_date":"..","next_expected":"..","active":bool,"account":"..."|null,"txn_ids":[...]}] }`
 - Group by (normalized payee, sign). Need >= 3 occurrences. Period = median gap in days snapped to weekly(7±1) / biweekly(14±2) /
   monthly(28-32) / quarterly(88-95) / yearly(360-370); require >= 70% of gaps within tolerance of the snapped period and amount CV <= 0.25.
   `active` = last_date within 1.5 periods of today (`AppState::today()`). Sort: active first, then annualized cost desc.
`GET /insights/anomalies` -> `{ "items":[{"kind":"amount_spike"|"duplicate"|"new_payee","txn_id":int,"date","payee","amount","detail":"3.4x this payee's median of $42.10"}] }`
 - amount_spike: payee (normalized, same sign) has >= 5 prior rows and the amount > median + 3*MAD-based sigma AND >= 2x median.
 - duplicate: same normalized payee + same amount + same account within 3 days of another row (report the later one).
 - new_payee: first-ever occurrence of a normalized payee in the last 30 days (relative to today) with amount >= 50.
 - Cap 200 items, newest first. Transfers (below) are excluded from all three.
`GET /insights/transfers` -> `{ "pairs":[{"out_id","in_id","amount","days_apart","out_account","in_account"}], "count":int, "total":float }`
 - One-to-one pairing: opposite signs, equal |amount| (to the cent), different accounts, within 4 days; greedy by smallest day gap then lowest id.
   Rows used by a pair are "transfer rows".

## Dashboard
`GET /dashboard?months=12&exclude_transfers=true` (months 1..60) ->
`{ "from":"YYYY-MM-01","to":"..","months":[{"month":"2026-10","out":float,"in":float,"net":float}],   // oldest first, zero-filled
   "by_category":[{"category_id":int|null,"path":"A › B"|"Uncategorized","out":float,"count":int}],      // top 15 by out, then one "Other" row (category_id null, path "Other") if more, Uncategorized always its own row if nonzero
   "top_payees":[{"payee":"..","out":float,"count":int}],                                                 // top 10 by out, by normalized payee
   "totals":{"out":float,"in":float,"net":float},
   "transfers_excluded":{"pairs":int,"total":float} }`
 - "out" sums positive amounts, "in" sums |negative amounts|. Top-level category rollup: `by_category` uses each row's own category path
   (leaf), not rolled up. Transfer rows excluded when exclude_transfers=true (default).

## Frontend (QML)
 - Dashboard view: monthly bars (out vs in), category bars with share %, top payees, the transfers-excluded note. `D` opens it (free key?
   verify against qml/keys/Keymap.js and pick a free one if taken; document in docs/KEYS.md and the help overlay). `Esc` closes.
 - Insights tabs inside the same overlay: Recurring / Anomalies / Transfers (`Tab`/`Shift-Tab` switch tabs, `j/k` move, `Enter` on an
   anomaly or series jumps the table to those rows via a search query, e.g. `p:"netflix"`; transfers jump to the pair).
 - Review queue overlay (key `R` if free): one row at a time with its options as numbered chips `1`/`2`/`3` = accept via
   `POST /transactions/categorize` (undoable, `u`), `c` opens the normal category picker, `s` skip, `Backspace` previous, `Esc` leave.
   Header shows mode ("Suggestions: top-3 accuracy 78% on 412 labeled" or "Bootstrap: biggest payees first - suggestions unlock after 30
   labeled rows and 70% top-3 accuracy (you have 3)"), remaining count, progress. In `frequency` mode, also offer `C` = the Phase 4
   smart-categorize on that row so a payee group is cleared in one step.
 - Detail pane: when suggestions are enabled, show the top option as a ghost chip; `Tab` while the table is focused... (do NOT steal Tab:
   it cycles panes) use `Ctrl-y` to accept. New overlays go through AppShell `restoreFocus()` and the overlayOpen list.
